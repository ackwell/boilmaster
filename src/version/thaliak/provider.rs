use std::{
	collections::{hash_map::Entry, HashMap},
	sync::{Arc, Mutex},
};

use anyhow::Result;
use either::Either;
use futures::future::try_join_all;
use graphql_client::{GraphQLQuery, Response};
use serde::Deserialize;
use tokio::sync::watch;

use crate::version::Patch;

// TODO: As-is this query can only fetch one repository per request. May be possible to programatically merge multiple into one query with a more struct-driven query system like cynic.
#[derive(GraphQLQuery)]
#[graphql(
	schema_path = "src/version/thaliak/schema.2022-08-14.json",
	query_path = "src/version/thaliak/query.graphql",
	response_derives = "Debug"
)]
pub struct RepositoryQuery;

type RepositoryData = repository_query::RepositoryQueryRepository;

#[derive(Debug, Deserialize)]
pub struct Config {
	endpoint: String,
}

#[derive(Debug)]
pub struct Provider {
	endpoint: String,
	client: reqwest::Client,

	data: Mutex<HashMap<String, Arc<watch::Sender<Option<Arc<RepositoryData>>>>>>,
}

impl Provider {
	pub fn new(config: Config) -> Self {
		Self {
			endpoint: config.endpoint,
			client: reqwest::Client::new(),

			data: Default::default(),
		}
	}

	/// Get the latest version for each of the specified repositories.
	pub async fn latest_versions(&self, repositories: Vec<String>) -> Result<Vec<String>> {
		tracing::info!("getting latest versions");

		let pending_repository_data = repositories
			.iter()
			// Bypassing the cache as the latest versions check should happen relatively
			// infrequently, and is a good opportunity to refresh the patch list.
			.map(|repository_name| self.repository_data(repository_name.to_owned(), true));

		let repository_data = try_join_all(pending_repository_data).await?;

		let latest = repository_data
			.iter()
			.map(|repository| repository.latest_version.version_string.clone())
			.collect::<Vec<_>>();

		Ok(latest)
	}

	/// Get the list of patch files for the specified repository that arrive at the specified version.
	pub async fn patch_list(&self, repository: String, version: &str) -> Result<Vec<Patch>> {
		let repository_data = self.repository_data(repository, false).await?;

		// Build a lookup of versions by their name.
		let versions = repository_data
			.versions
			.iter()
			.map(|version| (version.version_string.clone(), version))
			.collect::<HashMap<_, _>>();

		// TODO: this next_version handling effectively results in erroneous links causing empty or partial patch lists. consider if that's a problem.
		let mut patches = vec![];
		let mut next_version = versions.get(version).copied();

		while let Some(version) = next_version {
			// Get this version's patch - if there's anything other than exactly one patch, something has gone funky.
			let patch = match version.patches.as_slice() {
				[patch] => patch,
				_ => todo!("TODO: what even would cause this? i def. need to handle this as an exceptional case."),
			};

			// Record this patch file.
			patches.push(Patch {
				name: version.version_string.clone(),
				url: patch.url.clone(),
				size: patch.size.try_into().unwrap(),
			});

			// Grab the prerequsite versions data, split along is_active - we'll always
			// priotitise selecting active versions.
			let (mut active_versions, inactive_versions) = version
				.prerequisite_versions
				.iter()
				.filter_map(|specifier| {
					// Skip any patches that we've already seen, in case there's a dependency cycle.
					if patches
						.iter()
						.any(|patch| patch.name == specifier.version_string)
					{
						return None;
					}

					versions.get(&specifier.version_string)
				})
				.partition::<Vec<&repository_query::RepositoryQueryRepositoryVersions>, _>(
					|version| version.is_active,
				);

			// TODO: What does >1 active version imply? It seems to occur in places where it implies skipping a whole bunch of intermediary patches - i have to assume hotfixes. Is it skipping a bunch of .exe updates because they get bundled into the next main patch file as well?
			// It seems like it _can_ just be a bug; for sanity purposes, we're sorting
			// the array first to ensure that the "newest" active version is picked to
			// avoid accidentally skipping a bunch of patches. Patch names are string-sortable.
			active_versions.sort_by(|a, b| a.version_string.cmp(&b.version_string).reverse());

			// Try to select the active version to record next. If the current version
			// is inactive, allow falling back to inactive prerequesites as well.
			next_version = active_versions.first().cloned();
			if !version.is_active {
				next_version = next_version.or_else(|| inactive_versions.first().cloned());
			}
		}

		// Ironworks expects patches to be specified oldest-first - building down
		// from latest is the opposite of that, obviously, so fix that up.
		patches.reverse();

		Ok(patches)
	}

	/// Fetch data for the specified repository from thaliak. Unless `bypass_cache: true`
	/// is passed, values will be retrieved from a cache if available.
	// TODO: it's less bypassing and more invalidating - tempted to say that it should be a seperate call to be really explicit about that. Would require two mutex locks though.
	async fn repository_data(
		&self,
		repository: String,
		bypass_cache: bool,
	) -> Result<Arc<RepositoryData>> {
		let mut data_guard = self.data.lock().expect("poisoned");

		// If bypassing the cache, eagerly clear it.
		if bypass_cache {
			data_guard.remove(&repository);
		}

		// Get the data for the requested repository, taking responsibility for
		// fetching if it is not yet available.
		let channel = match data_guard.entry(repository.clone()) {
			Entry::Occupied(entry) => Either::Left(entry.get().clone()),
			Entry::Vacant(entry) => {
				let (sender, _receiver) = watch::channel(None);
				Either::Right(entry.insert(Arc::new(sender)).clone())
			}
		};

		// Eagerly drop the reference to the data mutex so other tasks can check it.
		drop(data_guard);

		match channel {
			// If there's alreay an entry, wait for a value to arrive.
			Either::Left(sender) => {
				// If there's already an available value, skip subscribing and return immediately.
				if let Some(data) = &*sender.borrow() {
					tracing::debug!("{repository}: GOT EXISTING");
					return Ok(data.clone());
				}

				// Otherwise, subscribe and wait for a value.
				// TODO: is it worth a cancellation here? not really?
				tracing::debug!("{repository}: WAITING");
				let mut receiver = sender.subscribe();
				while receiver.changed().await.is_ok() {
					if let Some(data) = &*receiver.borrow() {
						tracing::debug!("{repository}: GOT WAITING");
						return Ok(data.clone());
					}
				}

				// TODO: Should this be a regular error?
				panic!("Reciever failed but data hasn't resolved yet.")
			}

			// There wasn't an entry - we now have responsibility to fetch the data.
			Either::Right(sender) => {
				tracing::debug!("{repository}: RESPONSIBILITY ACQUIRED");

				// Request the data from thaliak.
				let query =
					RepositoryQuery::build_query(repository_query::Variables { repository });

				let response = self
					.client
					.post(&self.endpoint)
					.json(&query)
					.send()
					.await?
					.json::<Response<repository_query::ResponseData>>()
					.await?;

				// Check we actually got non-erroneous data.
				if let Some(errors) = response.errors {
					anyhow::bail!("TODO: thaliak errors: {errors:?}");
				}

				let data = response
					.data
					.and_then(|data| data.repository)
					.ok_or_else(|| anyhow::anyhow!("TODO: no data from thaliak"))?;

				// Save the value out for use and return it.
				let arc_data = Arc::new(data);
				sender.send_replace(Some(arc_data.clone()));

				return Ok(arc_data);
			}
		};
	}
}

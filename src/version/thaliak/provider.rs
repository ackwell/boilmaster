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
	// pub async fn patch_list(&self, repository: &str, version: &str) -> Result<()> {
	// 	// get the versions for the repository
	// 	// check that the version specified exists in our current view
	// 	// if it doesn't, invalidate + re-fetch
	// 	// if it still doesn't, fail out
	// 	// resolve to root patch and return
	// 	Ok(())
	// }

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

use std::collections::HashMap;

use anyhow::Result;
use graphql_client::{GraphQLQuery, Response};
use serde::Deserialize;

use crate::version::Patch;

#[derive(Debug, Deserialize)]
pub struct Config {
	endpoint: String,
}

// TODO: As-is this query can only fetch one repository per request. May be possible to programatically merge multiple into one query with a more struct-driven query system like cynic.
#[derive(GraphQLQuery)]
#[graphql(
	schema_path = "src/version/thaliak/schema.2022-08-14.json",
	query_path = "src/version/thaliak/query.graphql",
	response_derives = "Debug"
)]
pub struct RepositoryQuery;

#[derive(Debug)]
pub struct Provider {
	config: Config,
	client: reqwest::Client,
}

impl Provider {
	pub fn new(config: Config) -> Self {
		Self {
			config,
			client: reqwest::Client::new(),
		}
	}

	// TODO: how does versioning fall in on this? patch _lists_ technically sit above the concept of versions, but SE has a habit of "deprecating" patch files, which means that a patch list is only ever a point-in-time snapshot. Given that, I'm tempted to say that short term patch list should just represent "latest", and when i get around to actually building versioning, a version should "snapshot" the patch list at the time it's created (or configured or whatever) for reproducibility in the repository data cache.
	pub async fn patches(&self, repository: String) -> Result<Vec<Patch>> {
		// Request data from Thaliak.
		let query = RepositoryQuery::build_query(repository_query::Variables { repository });

		let response = self
			.client
			.post(&self.config.endpoint)
			.json(&query)
			.send()
			.await?
			.json::<Response<repository_query::ResponseData>>()
			.await?;

		if let Some(errors) = response.errors {
			anyhow::bail!("TODO: thaliak errors: {errors:?}")
		}

		let repository = response
			.data
			.and_then(|data| data.repository)
			.ok_or_else(|| anyhow::anyhow!("TODO: no data from thaliak"))?;

		// Build a mapping of versions by their string ID.
		let versions = repository
			.versions
			.into_iter()
			.map(|version| (version.version_string.clone(), version))
			.collect::<HashMap<_, _>>();

		// TODO: this next_version handling effectively results in erroneous links causing empty or partial patch lists. consider if that's a problem.
		let mut patches = vec![];
		let mut next_version = versions.get(&repository.latest_version.version_string);

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
				.partition::<Vec<_>, _>(|version| version.is_active);

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
}

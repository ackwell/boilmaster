use std::collections::HashMap;

use anyhow::Result;
use graphql_client::{GraphQLQuery, Response};
use nonempty::NonEmpty;
use serde::Deserialize;

#[derive(Debug)]
pub struct Patch {
	pub name: String,
	pub url: String,
	pub size: u64,
	// TODO: hashes (needs fixes @ thaliak)
}

// TODO: As-is this query can only fetch one repository per request. May be possible to programatically merge multiple into one query with a more struct-driven query system like cynic.
#[derive(GraphQLQuery)]
#[graphql(
	schema_path = "src/thaliak/schema.2022-08-14.json",
	query_path = "src/thaliak/query.graphql",
	response_derives = "Debug"
)]
struct RepositoryQuery;

#[derive(Debug, Deserialize)]
pub struct Config {
	endpoint: String,
	// {repository: {version: next_version}}
	overrides: Option<HashMap<String, HashMap<String, String>>>,
}

pub struct Provider {
	endpoint: String,
	overrides: HashMap<String, HashMap<String, String>>,
	client: reqwest::Client,
}

impl Provider {
	pub fn new(config: Config) -> Self {
		Self {
			endpoint: config.endpoint,
			overrides: config.overrides.unwrap_or_default(),
			client: reqwest::Client::new(),
		}
	}

	#[tracing::instrument(level = "debug", skip(self))]
	pub async fn patch_list(&self, repository: String) -> Result<NonEmpty<Patch>> {
		let query = RepositoryQuery::build_query(repository_query::Variables {
			repository: repository.clone(),
		});

		let response = self
			.client
			.post(&self.endpoint)
			.json(&query)
			.send()
			.await?
			.json::<Response<repository_query::ResponseData>>()
			.await?;

		if let Some(errors) = response.errors {
			anyhow::bail!("TODO: thaliak errors: {errors:?}")
		}

		let data = response
			.data
			.and_then(|data| data.repository)
			.ok_or_else(|| anyhow::anyhow!("received no data for repository \"{repository}\""))?;

		// Build a lookup of versions by their name string.
		let versions = data
			.versions
			.iter()
			.map(|version| (&version.version_string, version))
			.collect::<HashMap<_, _>>();

		let overrides = self.overrides.get(&repository);

		// TODO: this next_version handling effectively results in erroneous links causing empty or partial patch lists. consider if that's a problem. (it is)
		let mut patches = vec![];
		let mut next_version = versions.get(&data.latest_version.version_string).copied();

		while let Some(version) = next_version {
			// Get this version's patch file data.
			let patch = match version.patches.as_slice() {
				[patch] => patch,
				patches @ [patch, ..] => {
					tracing::warn!(?patches, "received >1 patch in a version");
					patch
				}
				[] => anyhow::bail!("no patches for version {}", version.version_string),
			};

			// Record this patch.
			patches.push(Patch {
				name: version.version_string.clone(),
				url: patch.url.clone(),
				size: patch.size.try_into().unwrap(),
			});

			// If there's an override for this version, use that and skip checking the active patches.
			if let Some(next_version_string) =
				overrides.and_then(|x| x.get(&version.version_string))
			{
				match versions.get(&next_version_string) {
					None => {
						tracing::warn!(
							current = version.version_string,
							next = next_version_string,
							"next version manual override not found, falling back to default behavior"
						);
					}

					Some(version) => {
						next_version = Some(version);
						continue;
					}
				}
			}

			// Grab the prerequsite versions, ignoring any that we've seen (to avoid
			// dependency cycles), or that are inactive (to avoid deprecated patches).
			let mut active_versions = version
				.prerequisite_versions
				.iter()
				.filter(|s| !patches.iter().any(|patch| patch.name == s.version_string))
				.filter_map(|specifier| versions.get(&specifier.version_string))
				.filter(|version| version.is_active)
				.copied()
				.collect::<Vec<_>>();

			// TODO: What does >1 active version imply? It seems to occur in places where it implies skipping a whole bunch of intermediary patches - i have to assume hotfixes. Is it skipping a bunch of .exe updates because they get bundled into the next main patch file as well?
			// It seems like it _can_ just be a bug; for sanity purposes, we're sorting
			// the array first to ensure that the "newest" active version is picked to
			// avoid accidentally skipping a bunch of patches. Patch names are string-sortable.
			active_versions.sort_by(|a, b| a.version_string.cmp(&b.version_string).reverse());

			next_version = active_versions.first().cloned()
		}

		// WORKAROUND: Around patches, when the patch servers are down, thaliak
		// blindly marks almost the entire patch chain as inactive, slash breaks the
		// chain. This doesn't last for long, but completely breaks any `latest`s
		// present at the time it occurs. To avoid this, we're failing out if
		// there's 1 or fewer patches in the chain - the only time this would ever
		// genuinely occur is on expansion release, and (as so far) no expansion
		// release has actually had just one patch file.
		if patches.len() <= 1 {
			anyhow::bail!("Thaliak returned a single-patch chain for {repository}");
		}

		// Ironworks expects patches to be specified oldest-first - building down
		// from latest is the opposite of that, obviously, so fix that up.
		patches.reverse();

		NonEmpty::from_vec(patches).ok_or_else(|| {
			anyhow::anyhow!(
				"could not build patch list for {repository} starting at {}",
				data.latest_version.version_string
			)
		})
	}
}

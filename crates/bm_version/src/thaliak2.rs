use anyhow::{Context, Result};
use nonempty::NonEmpty;
use serde::Deserialize;

use super::source;

#[derive(Debug, Deserialize)]
struct PatchesResponse {
	patches: Vec<Patch>,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
	version_string: String,
	remote_url: String,
	size: u64,
	// TODO: record + use hash
}

#[derive(Debug, Deserialize)]
pub struct Config {
	base: String,
}

pub struct Thaliak2 {
	base: String,
	client: reqwest::Client,
}

impl Thaliak2 {
	pub fn new(config: Config) -> Self {
		Self {
			base: config.base,
			client: reqwest::Client::new(),
		}
	}

	#[tracing::instrument(level = "debug", skip(self))]
	pub async fn patch_list(&self, repository: &str) -> Result<NonEmpty<source::Patch>> {
		let response = self
			.client
			.get(format!("{}/repositories/{repository}/patches", self.base))
			.send()
			.await?
			.json::<PatchesResponse>()
			.await?;

		let patches = response.patches.into_iter().map(|patch| source::Patch {
			name: patch.version_string,
			url: patch.remote_url,
			size: patch.size,
		});

		NonEmpty::collect(patches).context("no patches found")
	}
}

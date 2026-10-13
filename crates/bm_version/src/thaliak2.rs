use anyhow::{Context, Result};
use nonempty::NonEmpty;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PatchesResponse {
	patches: Vec<Patch>,
}

#[derive(Debug, Deserialize)]
pub struct Patch {
	#[serde(rename = "version_string")]
	pub name: String,
	#[serde(rename = "remote_url")]
	pub url: String,
	pub size: u64,
	// TODO: record + use hash
}

#[derive(Debug, Deserialize)]
pub struct Config {
	base: String,
}

pub struct Provider {
	base: String,
	client: reqwest::Client,
}

impl Provider {
	pub fn new(config: Config) -> Self {
		Self {
			base: config.base,
			client: reqwest::Client::new(),
		}
	}

	#[tracing::instrument(level = "debug", skip(self))]
	pub async fn patch_list(&self, repository: &str) -> Result<NonEmpty<Patch>> {
		let response = self
			.client
			.get(format!("{}/repositories/{repository}/patches", self.base))
			.send()
			.await?
			.json::<PatchesResponse>()
			.await?;

		NonEmpty::from_vec(response.patches).context("no patches found")
	}
}

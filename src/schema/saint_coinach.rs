use anyhow::Result;
use figment::value::magic::RelativePathBuf;
use ironworks_schema::{saint_coinach, Schema};
use serde::Deserialize;

use super::provider::Source;

#[derive(Debug, Deserialize)]
pub struct Config {
	remote: Option<String>,
	directory: RelativePathBuf,
}

pub struct SaintCoinach {
	provider: saint_coinach::Provider,
}

impl SaintCoinach {
	pub fn new(config: Config) -> Result<Self> {
		let mut builder = saint_coinach::Provider::with().directory(config.directory.relative());
		if let Some(remote) = config.remote {
			builder = builder.remote(remote);
		}

		Ok(Self {
			provider: builder.build()?,
		})
	}
}

impl Source for SaintCoinach {
	fn version(&self, version: Option<&str>) -> Result<Box<dyn Schema + '_>> {
		// TODO: the schema handler currently has absolutely no means for updating the repository once it's been cloned, so HEAD here will simply be "the position of HEAD at the time the system cloned the repository". Will need to build update mechanisms into stc's provider, and work out how I want to expose that here - it may be a better idea long-term to store the canonical reference for HEAD at the time of the latest update as a field locally?

		let version = self.provider.version(version.unwrap_or("HEAD"))?;

		Ok(Box::new(version))
	}
}
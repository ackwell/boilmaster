use anyhow::Result;
use figment::value::magic::RelativePathBuf;
use ironworks_schema::{saint_coinach, Schema};
use serde::Deserialize;

use crate::version::VersionKey;

use super::{error::Error, provider::Source};

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
	// TODO: should make this actually resolve a git hash properly from stc &c, but that'd require changes in iw and i'm being lazy at the moment. resolving it to our local defalt Should Do Fine for now.
	fn canonicalize(
		&self,
		schema_version: Option<&str>,
		_version_key: VersionKey,
	) -> Result<String, Error> {
		// TODO: the default version might be worth specifying in config?
		// TODO: the schema handler currently has absolutely no means for updating the repository once it's been cloned, so HEAD here will simply be "the position of HEAD at the time the system cloned the repository". Will need to build update mechanisms into stc's provider, and work out how I want to expose that here - it may be a better idea long-term to store the canonical reference for HEAD at the time of the latest update as a field locally?
		Ok(schema_version.unwrap_or("HEAD").to_string())
	}

	fn version(&self, version: &str) -> Result<Box<dyn Schema>, Error> {
		let version = self.provider.version(version)?;

		Ok(Box::new(version))
	}
}

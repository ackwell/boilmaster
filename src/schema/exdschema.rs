use std::{borrow::Cow, sync::Arc};

use anyhow::anyhow;
use ironworks_schema::exdschema;
use serde::Deserialize;

use crate::{data, utility::anyhow::Anyhow, version::VersionKey};

use super::{
	error::{Error, Result},
	provider::Source,
};

#[derive(Debug, Deserialize)]
pub struct Config {
	default: String,
	remote: String,
	directory: String,
}

pub struct ExdSchema {
	data: Arc<data::Data>,

	provider: exdschema::Provider,

	default: String,
}

impl ExdSchema {
	pub fn new(config: Config, data: Arc<data::Data>) -> Result<Self> {
		let provider = exdschema::Provider::with()
			.remote(config.remote)
			.directory(config.directory)
			.build()?;

		Ok(Self {
			data,
			provider,
			default: config.default,
		})
	}
}

impl Source for ExdSchema {
	fn update(&self) -> Result<()> {
		if self.provider.update()? {
			tracing::info!("EXDSchema updated")
		}
		Ok(())
	}

	fn canonicalize(
		&self,
		schema_version: Option<&str>,
		version_key: VersionKey,
	) -> Result<String> {
		let schema_version = schema_version.unwrap_or(&self.default);

		let split = schema_version.splitn(2, '-').collect::<Vec<_>>();
		let (reference, game_version) = match split[..] {
			[reference, game_version] => (reference, Cow::Borrowed(game_version)),
			// Errors here are effectively a full failure, we need the game version to resolve within the schema
			[reference] => (
				reference,
				self.data
					.version(version_key)
					.anyhow()?
					.excel()
					.version()
					.anyhow()?
					.into(),
			),
			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		};

		let specifier = self.provider.specifier(reference, &game_version)?;

		Ok(format!(
			"{}-{}",
			specifier.reference(),
			specifier.game_version()
		))
	}

	fn version(&self, version: &str) -> Result<Box<dyn ironworks_schema::Schema>> {
		let (reference, game_version) = version.split_once('-').ok_or_else(|| {
			Error::Failure(anyhow!("invalid canonical version string: \"{version}\""))
		})?;
		let specifier = self.provider.specifier(reference, game_version)?;
		let schema = self.provider.version(specifier)?;
		Ok(Box::new(schema))
	}
}

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
			.cache(true)
			.build()?;

		Ok(Self {
			data,
			provider,
			default: config.default,
		})
	}
}

impl Source for ExdSchema {
	fn ready(&self) -> bool {
		// The backing git repository is cloned as part of `::new`, so if this is
		// being called, we should be ready already.
		true
	}

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
		let specifier = self.specifier(schema_version, version_key)?;
		let string = match specifier {
			exdschema::Specifier::V1(v1) => format!("{}-{}", v1.reference(), v1.game_version()),
			exdschema::Specifier::V2(v2) => format!("2:ref:{}", v2.reference()),
		};
		Ok(string)
	}

	fn version(&self, version: &str) -> Result<Box<dyn ironworks_schema::Schema + Send>> {
		// TODO: This is stupid, but it works. Would be preferable to avoid needing
		// to re-fetch the specifier at all.
		let specifier = if version.starts_with("2:ref:") {
			self.provider.specifier_v2_ref(&version[6..])?
		} else {
			let (reference, game_version) = version.split_once('-').ok_or_else(|| {
				Error::Failure(anyhow!("invalid canonical version string: \"{version}\""))
			})?;
			self.provider.specifier_v1(reference, game_version)?
		};

		let schema = self.provider.version(specifier)?;
		Ok(Box::new(schema))
	}
}

impl ExdSchema {
	fn specifier(
		&self,
		schema_version: Option<&str>,
		version_key: VersionKey,
	) -> Result<exdschema::Specifier> {
		let schema_version = schema_version.unwrap_or(&self.default);

		// NOTE: The choice of `:` as the version specifier is very intentional -
		// it's an invalid character in git refs. Given the syntax for exds1
		// specifiers begins with a git ref, this (practically) ensures that
		// specifiers that include a valid `:` usage are not an exds1 specifier.
		let split = schema_version.splitn(2, ':').collect::<Vec<_>>();
		match split[..] {
			[v1_version] => self.specifier_v1(v1_version, version_key),
			["2", v2_version] => self.specifier_v2(v2_version, version_key),
			// Unknown version prefix, fail soft.
			[_exds_version, _spec] => Err(Error::InvalidVersion(schema_version.into())),
			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		}
	}

	fn specifier_v1(
		&self,
		schema_version: &str,
		version_key: VersionKey,
	) -> Result<exdschema::Specifier> {
		let split = schema_version.splitn(2, '-').collect::<Vec<_>>();
		let (reference, game_version) = match split[..] {
			[reference, game_version] => (reference, Cow::Borrowed(game_version)),
			// Errors here are effectively a full failure, we need the game version to resolve within the schema
			// TODO: Once the dust settles, consider making this try exds2, and fall back to exds1 if 2 fails.
			[reference] => (reference, self.excel_version(version_key)?.into()),
			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		};

		Ok(self.provider.specifier_v1(reference, &game_version)?)
	}

	fn specifier_v2(
		&self,
		schema_version: &str,
		version_key: VersionKey,
	) -> Result<exdschema::Specifier> {
		let split = schema_version.splitn(2, ':').collect::<Vec<_>>();
		let specifier = match split[..] {
			// No prefix should be treated as a ref
			[reference] | ["ref", reference] => self.provider.specifier_v2_ref(reference)?,

			["ver", "request"] => self
				.provider
				.specifier_v2_ver(&self.excel_version(version_key)?)?,

			["ver", version] => self.provider.specifier_v2_ver(version)?,

			// Unknown prefix, fail soft.
			[_, _] => return Err(Error::InvalidVersion(schema_version.into())),

			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		};
		Ok(specifier)
	}

	fn excel_version(&self, version_key: VersionKey) -> Result<String> {
		let version = self
			.data
			.version(version_key)
			.anyhow()?
			.excel()
			.version()
			.anyhow()?;

		Ok(version)
	}
}

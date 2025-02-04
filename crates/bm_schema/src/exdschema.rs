use std::{borrow::Cow, sync::Arc};

use anyhow::anyhow;
use bm_version::VersionKey;
use ironworks_schema::exdschema;
use serde::Deserialize;

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
	data: Arc<bm_data::Data>,

	provider: exdschema::Provider,

	default: String,
}

impl ExdSchema {
	pub fn new(config: Config, data: Arc<bm_data::Data>) -> Result<Self> {
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
			exdschema::Specifier::V1(v1) => format!("{}-{}", v1.revision(), v1.game_version()),
			exdschema::Specifier::V2(v2) => format!("2:rev:{}", v2.revision()),
		};
		Ok(string)
	}

	fn version(&self, version: &str) -> Result<Box<dyn ironworks_schema::Schema + Send>> {
		// TODO: This is stupid, but it works. Would be preferable to avoid needing
		// to re-fetch the specifier at all.
		let specifier = if version.starts_with("2:rev:") {
			self.provider.specifier_v2_rev(&version[6..])?
		} else {
			let (revision, game_version) = version.split_once('-').ok_or_else(|| {
				Error::Failure(anyhow!("invalid canonical version string: \"{version}\""))
			})?;
			self.provider.specifier_v1(revision, game_version)?
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
		// it's an invalid character in git commit revs. Given the syntax for exds1
		// specifiers begins with a git rev (ostensibly targeting a commit), this
		// (practically) ensures that specifiers that include a valid `:` usage are
		// not an exds1 specifier.
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
		let (revision, game_version) = match split[..] {
			[revision, game_version] => (revision, Cow::Borrowed(game_version)),
			// Errors here are effectively a full failure, we need the game version to resolve within the schema
			// TODO: Once the dust settles, consider making this try exds2, and fall back to exds1 if 2 fails.
			[revision] => (revision, self.excel_version(version_key)?.into()),
			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		};

		Ok(self.provider.specifier_v1(revision, &game_version)?)
	}

	fn specifier_v2(
		&self,
		schema_version: &str,
		version_key: VersionKey,
	) -> Result<exdschema::Specifier> {
		let split = schema_version.splitn(2, ':').collect::<Vec<_>>();
		let specifier = match split[..] {
			// No prefix should be treated as a rev
			[revision] | ["rev", revision] => self.provider.specifier_v2_rev(revision)?,

			["ver", "request"] => self
				.provider
				.specifier_v2_ver(&self.excel_version(version_key)?)?,

			// Removing for now: This is expecting consumers to provide a valid _excel_ version, not a boilmaster-understood version. Just confusing, revisit this.
			// ["ver", version] => self.provider.specifier_v2_ver(version)?,

			// Unknown prefix, fail soft.
			[_, _] => return Err(Error::InvalidVersion(schema_version.into())),

			_ => unreachable!("splitn should ensure this vec contains 1 or 2 entries"),
		};
		Ok(specifier)
	}

	fn excel_version(&self, version_key: VersionKey) -> Result<String> {
		let version = self.data.version(version_key)?.excel().version()?;

		Ok(version)
	}
}

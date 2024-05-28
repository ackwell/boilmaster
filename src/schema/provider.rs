use std::{collections::HashMap, sync::Arc};

use ironworks_schema::Schema;
use serde::Deserialize;

use crate::{data, version::VersionKey};

use super::{
	error::{Error, Result},
	exdschema, saint_coinach,
	specifier::CanonicalSpecifier,
	Specifier,
};

pub trait Source: Send + Sync {
	fn canonicalize(&self, schema_version: Option<&str>, version_key: VersionKey)
		-> Result<String>;

	fn version(&self, version: &str) -> Result<Box<dyn Schema>>;
}

#[derive(Debug, Deserialize)]
pub struct Config {
	default: Specifier,

	exdschema: exdschema::Config,
	saint_coinach: saint_coinach::Config,
}

// TODO: need a way to handle updating the repo
// TODO: look into moving sources into a channel so i'm not leaning on send+sync for other shit
pub struct Provider {
	default: Specifier,
	sources: HashMap<&'static str, Box<dyn Source>>,
}

impl Provider {
	pub fn new(config: Config, data: Arc<data::Data>) -> Result<Self> {
		// TODO: at the moment this will hard fail if any source fails - should i make sources soft fail?
		Ok(Self {
			default: config.default,
			sources: HashMap::from([
				(
					"saint-coinach",
					boxed(saint_coinach::SaintCoinach::new(config.saint_coinach)?),
				),
				(
					"exdschema",
					boxed(exdschema::ExdSchema::new(config.exdschema, data)?),
				),
			]),
		})
	}

	/// Canonicalise an optional specifier.
	pub fn canonicalize(
		&self,
		specifier: Option<Specifier>,
		version: VersionKey,
	) -> Result<CanonicalSpecifier> {
		let specifier = specifier.unwrap_or_else(|| self.default.clone());

		let source = self
			.sources
			.get(specifier.source.as_str())
			.ok_or_else(|| Error::UnknownSource(specifier.source.clone()))?;

		Ok(CanonicalSpecifier {
			source: specifier.source,
			version: source.canonicalize(specifier.version.as_deref(), version)?,
		})
	}

	pub fn schema(&self, specifier: CanonicalSpecifier) -> Result<Box<dyn Schema>> {
		let source = self
			.sources
			.get(specifier.source.as_str())
			.ok_or_else(|| Error::UnknownSource(specifier.source.clone()))?;
		source.version(&specifier.version)
	}
}

fn boxed(x: impl Source + 'static) -> Box<dyn Source> {
	Box::new(x)
}

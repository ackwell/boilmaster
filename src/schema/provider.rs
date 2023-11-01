use std::collections::HashMap;

use ironworks_schema::Schema;
use serde::Deserialize;

use super::{error::Error, saint_coinach, specifier::CanonicalSpecifier, Specifier};

pub trait Source: Send + Sync {
	fn canonicalize(&self, version: Option<&str>) -> Result<String, Error>;
	fn version(&self, version: &str) -> Result<Box<dyn Schema>, Error>;
}

#[derive(Debug, Deserialize)]
pub struct Config {
	default: Specifier,

	saint_coinach: saint_coinach::Config,
}

// TODO: need a way to handle updating the repo
pub struct Provider {
	default: Specifier,
	sources: HashMap<&'static str, Box<dyn Source>>,
}

impl Provider {
	pub fn new(config: Config) -> Result<Self, Error> {
		// TODO: at the moment this will hard fail if any source fails - should i make sources soft fail?
		Ok(Self {
			default: config.default,
			sources: HashMap::from([(
				"saint-coinach",
				boxed(saint_coinach::SaintCoinach::new(config.saint_coinach)?),
			)]),
		})
	}

	/// Canonicalise an optional specifier.
	pub fn canonicalize(&self, specifier: Option<Specifier>) -> Result<CanonicalSpecifier, Error> {
		let specifier = specifier.unwrap_or_else(|| self.default.clone());

		let source = self
			.sources
			.get(specifier.source.as_str())
			.ok_or_else(|| Error::UnknownSource(specifier.source.clone()))?;

		Ok(CanonicalSpecifier {
			source: specifier.source,
			version: source.canonicalize(specifier.version.as_deref())?,
		})
	}

	pub fn schema(&self, specifier: CanonicalSpecifier) -> Result<Box<dyn Schema>, Error> {
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

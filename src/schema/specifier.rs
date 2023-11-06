use std::{convert::Infallible, str::FromStr};

use serde::{de, Deserialize};

// TODO: will probably need eq/hash so i can use these as cache keys?
#[derive(Debug)]
pub struct CanonicalSpecifier {
	pub(super) source: String,
	pub(super) version: String,
}

impl CanonicalSpecifier {
	pub fn source(&self) -> &str {
		&self.source
	}

	pub fn version(&self) -> &str {
		&self.version
	}
}

#[derive(Debug, Clone)]
pub struct Specifier {
	pub source: String,
	pub version: Option<String>,
}

impl FromStr for Specifier {
	type Err = Infallible;

	fn from_str(string: &str) -> Result<Self, Self::Err> {
		let out = match string.split_once('@') {
			Some((source, version)) => Self {
				source: source.to_string(),
				version: Some(version.to_string()),
			},
			None => Self {
				source: string.to_string(),
				version: None,
			},
		};

		Ok(out)
	}
}

impl<'de> Deserialize<'de> for Specifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

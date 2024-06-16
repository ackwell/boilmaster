use std::{borrow::Cow, convert::Infallible, str::FromStr};

use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject, StringValidation},
	JsonSchema,
};
use serde::{de, Deserialize, Serialize};

// TODO: will probably need eq/hash so i can use these as cache keys?
#[derive(Debug, Clone)]
pub struct CanonicalSpecifier {
	pub source: String,
	pub version: String,
}

impl ToString for CanonicalSpecifier {
	fn to_string(&self) -> String {
		format!("{}@{}", self.source, self.version)
	}
}

impl Serialize for CanonicalSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(&self.to_string())
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

impl JsonSchema for Specifier {
	fn schema_name() -> String {
		"Specifier".into()
	}

	fn schema_id() -> Cow<'static, str> {
		Cow::Borrowed(concat!(module_path!(), "::Specifier"))
	}

	fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
		Schema::Object(SchemaObject {
			instance_type: Some(InstanceType::String.into()),
			string: Some(
				StringValidation {
					pattern: Some("^.+(@.+)?$".into()),
					..Default::default()
				}
				.into(),
			),
			..Default::default()
		})
	}
}

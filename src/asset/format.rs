use std::{borrow::Cow, str::FromStr};

use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject},
	JsonSchema,
};
use serde::{de, Deserialize, Serialize};
use strum::{EnumIter, IntoEnumIterator};

use super::{convert, error::Error};

#[derive(Debug, Clone, Copy, EnumIter)]
pub enum Format {
	Png,
}

impl Format {
	pub fn extension(&self) -> &str {
		match self {
			Self::Png => "png",
		}
	}

	pub(super) fn converter(&self) -> &dyn convert::Converter {
		match self {
			Self::Png => &convert::Image,
		}
	}
}

// NOTE: Changing the string format is breaking to API1 - isolate if doing so.
impl Serialize for Format {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.extension().serialize(serializer)
	}
}

impl FromStr for Format {
	type Err = Error;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		Ok(match input {
			"png" => Self::Png,
			other => return Err(Error::UnknownFormat(other.into())),
		})
	}
}

impl<'de> Deserialize<'de> for Format {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

impl JsonSchema for Format {
	fn schema_name() -> String {
		"Format".into()
	}

	fn schema_id() -> Cow<'static, str> {
		Cow::Borrowed(concat!(module_path!(), "::Format"))
	}

	fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
		Schema::Object(SchemaObject {
			instance_type: Some(InstanceType::String.into()),
			enum_values: Some(
				Format::iter()
					.map(|format| serde_json::to_value(format).expect("should not fail"))
					.collect(),
			),
			..Default::default()
		})
	}
}

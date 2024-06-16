use std::{fmt, str::FromStr};

use ironworks::excel::Language;
use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject},
};
use serde::de;

use crate::utility::jsonschema::impl_jsonschema;

use super::error::Error;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanguageString(Language);

impl fmt::Debug for LanguageString {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(formatter)
	}
}

impl fmt::Display for LanguageString {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let string = match self.0 {
			Language::None => "none",
			Language::Japanese => "ja",
			Language::English => "en",
			Language::German => "de",
			Language::French => "fr",
			Language::ChineseSimplified => "chs",
			Language::ChineseTraditional => "cht",
			Language::Korean => "kr",
		};
		formatter.write_str(string)
	}
}

impl From<LanguageString> for Language {
	fn from(wrapper: LanguageString) -> Self {
		wrapper.0
	}
}

impl From<Language> for LanguageString {
	fn from(inner: Language) -> Self {
		Self(inner)
	}
}

impl FromStr for LanguageString {
	type Err = Error;

	fn from_str(string: &str) -> Result<Self, Self::Err> {
		let language = match string {
			"none" => Language::None,
			"ja" => Language::Japanese,
			"en" => Language::English,
			"de" => Language::German,
			"fr" => Language::French,
			"chs" => Language::ChineseSimplified,
			"cht" => Language::ChineseTraditional,
			"kr" => Language::Korean,
			_ => return Err(Error::UnknownLanguage(string.into())),
		};

		Ok(Self(language))
	}
}

impl<'de> de::Deserialize<'de> for LanguageString {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

impl_jsonschema!(LanguageString, languagestring_schema);
fn languagestring_schema(_generator: &mut SchemaGenerator) -> Schema {
	// TODO: keep this up to date with the full list.
	let languages = [
		Language::None,
		Language::Japanese,
		Language::English,
		Language::German,
		Language::French,
		Language::ChineseSimplified,
		Language::ChineseTraditional,
		Language::Korean,
	];

	Schema::Object(SchemaObject {
		instance_type: Some(InstanceType::String.into()),
		enum_values: Some(
			languages
				.map(|language| LanguageString(language).to_string().into())
				.to_vec(),
		),
		..Default::default()
	})
}

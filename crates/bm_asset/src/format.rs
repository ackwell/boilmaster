use std::str::FromStr;

use serde::{de, Deserialize, Serialize};
use strum::{EnumIter, IntoEnumIterator};

use super::{convert, error::Error};

#[derive(Debug, Clone, Copy, EnumIter)]
pub enum Format {
	Jpeg,
	Png,
	Webp,
}

impl Format {
	pub fn iter() -> impl Iterator<Item = Format> {
		<Self as IntoEnumIterator>::iter()
	}

	pub fn extension(&self) -> &str {
		match self {
			Self::Jpeg => "jpg",
			Self::Png => "png",
			Self::Webp => "webp",
		}
	}

	pub(super) fn converter(&self) -> &dyn convert::Converter {
		match self {
			Self::Jpeg => &convert::Image,
			Self::Png => &convert::Image,
			Self::Webp => &convert::Image,
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
			"jpg" => Self::Jpeg,
			"png" => Self::Png,
			"webp" => Self::Webp,
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

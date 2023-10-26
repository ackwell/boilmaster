use std::{
	fmt,
	hash::{Hash, Hasher},
	num::ParseIntError,
	str::FromStr,
};

use seahash::SeaHasher;
use serde::{de, Deserialize, Serialize};

use super::version::Version;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VersionKey(u64);

impl From<&Version> for VersionKey {
	fn from(version: &Version) -> Self {
		let mut hasher = SeaHasher::new();

		for patch in version
			.repositories
			.iter()
			.map(|repository| repository.latest())
		{
			patch.name.hash(&mut hasher);
		}

		Self(hasher.finish())
	}
}

impl fmt::Display for VersionKey {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_fmt(format_args!("{:016x}", self.0))
	}
}

impl FromStr for VersionKey {
	type Err = ParseIntError;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		u64::from_str_radix(input, 16).map(VersionKey)
	}
}

impl Serialize for VersionKey {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(&self.to_string())
	}
}

impl<'de> Deserialize<'de> for VersionKey {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

use std::path::PathBuf;

use anyhow::Result;
use nonempty::NonEmpty;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, PartialEq)]
pub struct Version {
	pub repositories: Vec<Repository>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PersistedVersion {
	V2(PersistedVersionV2),
	V1(Vec<PersistedRepository>),
}

#[derive(Serialize, Deserialize)]
struct PersistedVersionV2 {
	repositories: Vec<PersistedRepository>,
}

// NOTE: This using using `impl Serialize` so it doesn't become public API surface.
impl Version {
	pub(super) fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok> {
		let persisted_version = PersistedVersionV2 {
			repositories: self
				.repositories
				.iter()
				.map(|repository| PersistedRepository {
					name: repository.name.clone(),
					patches: repository.patches.clone().map(|patch| patch.name),
				})
				.collect(),
		};

		PersistedVersion::V2(persisted_version)
			.serialize(serializer)
			.map_err(|err| anyhow::anyhow!(err.to_string()))
	}
}

impl Version {
	pub(super) fn deserialize<'de, D: Deserializer<'de>>(
		deserializer: D,
		get_path: impl Fn(&str, &str) -> PathBuf,
	) -> Result<Self> {
		let persisted_version = PersistedVersion::deserialize(deserializer)
			.map_err(|err| anyhow::anyhow!(err.to_string()))?;

		let persisted_repositories = match persisted_version {
			PersistedVersion::V1(repositories) => repositories,
			PersistedVersion::V2(version) => version.repositories,
		};

		let repositories = persisted_repositories
			.into_iter()
			.map(|persisted_repository| Repository {
				patches: persisted_repository.patches.map(|patch_name| Patch {
					// TODO: I should probably fail out if this doesn't point to a file on disk.
					path: get_path(&persisted_repository.name, &patch_name),
					name: patch_name,
				}),
				name: persisted_repository.name,
			})
			.collect();

		Ok(Version { repositories })
	}
}

#[derive(Clone, PartialEq)]
pub struct Repository {
	pub name: String,
	pub patches: NonEmpty<Patch>,
}

#[derive(Serialize, Deserialize)]
struct PersistedRepository {
	name: String,
	patches: NonEmpty<String>,
}

impl Repository {
	/// Get the most recent patch in the repository.
	pub fn latest(&self) -> &Patch {
		// Per IW semantics, patches are ordered oldest-first - get the last.
		self.patches.last()
	}
}

// NOTE: this _must_ be resolvable purely from local fs data, assuming the file exists
#[derive(Clone, PartialEq)]
pub struct Patch {
	pub name: String,
	pub path: PathBuf,
}

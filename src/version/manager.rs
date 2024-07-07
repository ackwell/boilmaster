use std::{
	collections::{hash_map::Entry, BTreeMap, HashMap},
	fs,
	io::{self, Read},
	path::{Path, PathBuf},
	sync::RwLock,
};

use anyhow::Result;
use figment::value::magic::RelativePathBuf;
use fs4::FileExt;
use futures::future::{join_all, try_join_all};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use tokio::{select, sync::broadcast, time};
use tokio_util::sync::CancellationToken;

use super::{
	key::VersionKey,
	patcher, thaliak,
	version::{Repository, Version},
};

const TAG_LATEST: &str = "latest";

#[derive(Debug, Deserialize)]
pub struct Config {
	thaliak: thaliak::Config,
	patch: patcher::Config,

	interval: u64,
	directory: RelativePathBuf,
	repositories: Vec<String>,
}

/// Messgages that may be broadcast by the version system.
#[derive(Debug, Clone)]
pub enum VersionMessage {
	/// Version data has been hydrated from disk. Version keys associated with
	/// this event will exist. It can be assumed that no change since previous
	/// messages has occured to these versions.
	Hydrate(Vec<VersionKey>),

	/// A version was added or updated. It should be assumed that some detail
	/// about the associated version has changed, and any caches of data are
	/// stale.
	Changed(VersionKey),
}

pub struct Manager {
	provider: thaliak::Provider,
	patcher: patcher::Patcher,

	update_interval: u64,
	directory: PathBuf,
	repositories: Vec<String>,

	versions: RwLock<HashMap<VersionKey, Version>>,
	names: RwLock<HashMap<String, VersionKey>>,

	channel: broadcast::Sender<VersionMessage>,
}

impl Manager {
	pub fn new(config: Config) -> Result<Self> {
		let directory = config.directory.relative();
		fs::create_dir_all(&directory)?;

		// Realistically; we're only going to signal one version at a time - 10 should be more than enough for our use cases.
		let (sender, _receiver) = broadcast::channel(10);

		Ok(Self {
			provider: thaliak::Provider::new(config.thaliak),
			patcher: patcher::Patcher::new(config.patch),

			update_interval: config.interval,
			directory,
			repositories: config.repositories,

			versions: Default::default(),
			names: Default::default(),

			channel: sender,
		})
	}

	pub fn ready(&self) -> bool {
		// Mark ready once we've got at least one version - existing systems will
		// hydrate metadata from disk in one go.
		self.versions.read().expect("poisoned").len() > 0
	}

	/// Subscribe to changes to the version list.
	pub fn subscribe(&self) -> broadcast::Receiver<VersionMessage> {
		self.channel.subscribe()
	}

	/// Get a list of all known version keys.
	pub fn keys(&self) -> Vec<VersionKey> {
		self.versions
			.read()
			.expect("poisoned")
			.keys()
			.copied()
			.collect()
	}

	/// Resolve a version name to its key, if the name is known. If no version is
	/// specified. the version marked as latest will be returned.
	pub fn resolve(&self, name: Option<&str>) -> Option<VersionKey> {
		self.names
			.read()
			.expect("poisoned")
			.get(name.unwrap_or(TAG_LATEST))
			.copied()
	}

	/// Get a list of all known version names.
	pub fn all_names(&self) -> Vec<String> {
		self.names
			.read()
			.expect("poisoned")
			.keys()
			.cloned()
			.collect()
	}

	/// Get a list of names for a given version key.
	pub fn names(&self, key: VersionKey) -> Option<Vec<String>> {
		// Make sure the version is actually known to exist, to distinguish between an unknown key and a key with no names.
		if !self.versions.read().expect("poisoned").contains_key(&key) {
			return None;
		}

		let names = self
			.names
			.read()
			.expect("poisoned")
			.iter()
			.filter_map(|(name, inner_key)| (*inner_key == key).then(|| name.clone()))
			.collect();

		Some(names)
	}

	/// Set the names for the specified version. If a name already exists, it
	/// will be updated to match.
	pub async fn set_names(
		&self,
		key: VersionKey,
		new_names: impl IntoIterator<Item = impl ToString>,
	) -> Result<()> {
		// Funny squigglies because something in the checker(s) doesn't manage to track ownership properly with a drop().
		{
			let mut names = self.names.write().expect("poisoned");
			names.retain(|_, value| *value != key);
			names.extend(new_names.into_iter().map(|name| (name.to_string(), key)));
		}
		self.persist_metadata().await?;
		Ok(())
	}

	/// Get the full version metadata for a given key, if it exists.
	pub fn version(&self, key: VersionKey) -> Option<Version> {
		self.versions.read().expect("poisoned").get(&key).cloned()
	}

	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		select! {
			result = self.start_inner() => result,
			_ = cancel.cancelled() => Ok(())
		}
	}

	async fn start_inner(&self) -> Result<()> {
		// Hydrate from disk.
		self.hydrate().await?;

		// Set up an interval to check for updates.
		let mut interval = time::interval(time::Duration::from_secs(self.update_interval));
		interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

		loop {
			interval.tick().await;

			if let Err(error) = self.update().await {
				tracing::error!(?error, "update failed");
			}
		}
	}

	// TODO: There should only be one update pass running at a time - two would result in races.
	async fn update(&self) -> Result<()> {
		tracing::info!("checking for version updates");

		// Get a fresh view of the repositories.
		let pending_repositories = self
			.repositories
			.iter()
			.map(|repository| self.fetch_repository(repository));
		let repositories = try_join_all(pending_repositories).await?;

		// Build a version struct and it's associated key and save it to the versions map.
		let version = Version { repositories };
		let key = VersionKey::from(&version);

		let mut versions = self.versions.write().expect("poisoned");

		let changed = match versions.entry(key) {
			// New version entry - mark it as latest and request an update.
			Entry::Vacant(entry) => {
				entry.insert(version.clone());
				true
			}

			// Existing entry, check if the requisite patches have changed before saving.
			Entry::Occupied(mut entry) => {
				let changed = *entry.get() != version;
				if changed {
					entry.insert(version.clone());
				}
				changed
			}
		};

		drop(versions);

		// If there hasn't been any changes from this update, skip running updates beyond this point.
		if !changed {
			return Ok(());
		}

		tracing::info!(%key, "new or updated version");

		// Update latest tag.
		// TODO: This might need to be moved to manual-only for now? If there's any long-running ingestion tasks (i.e. search) hanging off versions, then setting latest _now_ would leave end-consumers pointing at an uningested tag.
		self.names
			.write()
			.expect("poisoned")
			.insert(TAG_LATEST.to_string(), key);

		// Persist updated metadata
		tokio::try_join!(
			//
			self.persist_version(key, version),
			self.persist_metadata()
		)?;

		// There's a change to versions, broadcast as such.
		// We don't care if anyone is actually listening on the channel.
		let _ = self.channel.send(VersionMessage::Changed(key));

		Ok(())
	}

	async fn fetch_repository(&self, repository: &str) -> Result<Repository> {
		// a failure to fetch the patch list for a repo is pretty unrecoverable i think?
		let patch_list = self.provider.patch_list(repository.to_string()).await?;

		// todo: is a failure here meaningful? i imagine retries and so on should be done at the patcher
		// note: would use nonempty::map but i need asyncnessnessness
		let pending_patches = patch_list
			.into_iter()
			.map(|patch| self.patcher.to_local_patch(repository, patch));
		let patches = NonEmpty::from_vec(try_join_all(pending_patches).await?)
			.expect("non-empty list is guaranteed by provider");

		Ok(Repository {
			name: repository.to_string(),
			patches,
		})
	}

	fn metadata_path(&self) -> PathBuf {
		self.directory.join("metadata.json")
	}

	fn version_path(&self, key: VersionKey) -> PathBuf {
		self.directory.join(format!("version-{key}.json"))
	}

	async fn hydrate(&self) -> Result<()> {
		let Some(metadata) = self.hydrate_metadata().await? else {
			return Ok(());
		};

		let pending_versions = metadata
			.versions
			.iter()
			.map(|key| self.hydrate_version(*key));

		let hydrated_versions = join_all(pending_versions)
			.await
			.into_iter()
			.zip(metadata.versions);

		let mut versions = self.versions.write().expect("poisoned");

		for (result, key) in hydrated_versions {
			let version = match result {
				Ok(version) => version,
				Err(error) => {
					tracing::warn!(%key, ?error, "could not hydrate version");
					continue;
				}
			};

			tracing::debug!(%key, "hydrated version");
			versions.insert(key, version);
		}

		drop(versions);

		let versions = self.versions.read().expect("poisoned");
		let mut names = self.names.write().expect("poisoned");

		for (name, key) in metadata.names {
			if !versions.contains_key(&key) {
				tracing::warn!(name, %key, "unknown key for name");
				continue;
			}

			tracing::debug!(name, %key, "named version");
			names.insert(name, key);
		}

		// Hydration is complete - broadcast the version list.
		let keys = versions.keys().copied().collect::<Vec<_>>();
		let _ = self.channel.send(VersionMessage::Hydrate(keys));

		Ok(())
	}

	async fn hydrate_metadata(&self) -> Result<Option<PersistedMetadata>> {
		let path = self.metadata_path();
		let join_handle = tokio::task::spawn_blocking(|| -> Result<Option<PersistedMetadata>> {
			let Some(file) = open_config_read(path)? else {
				return Ok(None);
			};
			let metadata: PersistedMetadata = serde_json::from_reader(file)?;
			Ok(Some(metadata))
		});

		join_handle.await?
	}

	async fn hydrate_version(&self, key: VersionKey) -> Result<Version> {
		// NOTE: Parsing outside the task so I don't have to get the self reference into the task for patch paths.
		let path = self.version_path(key);
		let join_handle = tokio::task::spawn_blocking(move || -> Result<String> {
			let Some(mut file) = open_config_read(path)? else {
				anyhow::bail!("version {key} has no persisted configuration")
			};
			let mut buffer = String::new();
			file.read_to_string(&mut buffer)?;
			Ok(buffer)
		});
		let string_config = join_handle.await??;

		let version = Version::deserialize(
			&mut serde_json::Deserializer::from_str(&string_config),
			|repository, patch| self.patcher.patch_path(repository, patch),
		)?;

		// TODO: should probably validate these versions too - will need to store at least the file size, and preferably the hash as well once i have that.

		Ok(version)
	}

	async fn persist_metadata(&self) -> Result<()> {
		let persisted_versions = PersistedMetadata {
			versions: self
				.versions
				.read()
				.expect("poisoned")
				.keys()
				.copied()
				.collect(),

			names: self
				.names
				.read()
				.expect("poisoned")
				.clone()
				.into_iter()
				.collect(),
		};

		let path = self.metadata_path();
		let join_handle = tokio::task::spawn_blocking(move || -> Result<()> {
			let file = open_config_write(path)?;
			serde_json::to_writer_pretty(file, &persisted_versions)?;
			Ok(())
		});

		join_handle.await?
	}

	async fn persist_version(&self, key: VersionKey, version: Version) -> Result<()> {
		let path = self.directory.join(format!("version-{key}.json"));
		let join_handle = tokio::task::spawn_blocking(move || -> Result<()> {
			let file = open_config_write(path)?;
			version.serialize(&mut serde_json::Serializer::pretty(file))?;
			Ok(())
		});
		join_handle.await?
	}
}

#[derive(Serialize, Deserialize)]
struct PersistedMetadata {
	versions: Vec<VersionKey>,
	names: BTreeMap<String, VersionKey>,
}

fn open_config_read(path: impl AsRef<Path>) -> Result<Option<fs::File>> {
	let file = match fs::File::open(path) {
		Ok(file) => file,
		Err(error) => {
			return match error.kind() {
				io::ErrorKind::NotFound => Ok(None),
				_ => Err(error.into()),
			}
		}
	};

	file.lock_shared()?;

	Ok(Some(file))
}

fn open_config_write(path: impl AsRef<Path>) -> Result<fs::File> {
	let file = fs::File::options().create(true).write(true).open(path)?;
	file.lock_exclusive()?;
	file.set_len(0)?;
	Ok(file)
}

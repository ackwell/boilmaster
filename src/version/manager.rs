use std::{collections::HashMap, sync::RwLock};

use anyhow::{Context, Result};
use futures::future::try_join_all;
use serde::Deserialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{key::VersionKey, patch::Patch, thaliak, version::Version};

pub type PatchList = Vec<(String, Vec<Patch>)>;

#[derive(Debug, Deserialize)]
pub struct Config {
	thaliak: thaliak::Config,

	// interval
	// directory
	repositories: Vec<String>,
}

#[derive(Debug)]
pub struct Manager {
	thaliak: thaliak::Provider,

	channel: watch::Sender<Vec<VersionKey>>,

	repositories: Vec<String>,
	versions: RwLock<HashMap<VersionKey, Version>>,
}

impl Manager {
	pub fn new(config: Config) -> Result<Self> {
		let (sender, _receiver) = watch::channel(vec![]);

		let manager = Self {
			thaliak: thaliak::Provider::new(config.thaliak),

			channel: sender,

			repositories: config.repositories,
			versions: Default::default(),
		};

		Ok(manager)
	}

	/// Subscribe to changes to the version list.
	pub fn subscribe(&self) -> watch::Receiver<Vec<VersionKey>> {
		tracing::debug!("SUBSCRIBE");
		self.channel.subscribe()
	}

	/// Resolve a version name to its key. If no version is specified, the version marked as latest will be returned, if any exists.
	pub fn resolve(&self, name: Option<&str>) -> Option<VersionKey> {
		todo!("resolve {name:?}")
	}

	/// Get a patch list for a given version.
	pub fn patch_list(&self, key: VersionKey) -> Result<PatchList> {
		let versions = self.versions.read().expect("poisoned");
		let version = versions
			.get(&key)
			.with_context(|| format!("unknown version {key}"))?;

		// TODO: A version made on repository list [a, b] will create a patch list [a] on an updated repository list of [a, X, b]. I'm not convinced that's a problem.
		let patch_list = self
			.repositories
			.iter()
			.map_while(|repository_name| {
				version
					.patches()
					.get(repository_name)
					.map(|patches| (repository_name.clone(), patches.clone()))
			})
			.collect::<Vec<_>>();

		Ok(patch_list)
	}

	/// Start the service.
	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		// TODO: timer and all that shmuck
		self.update().await?;

		Ok(())
	}

	/// Run an update of the data, fetching version data from provider(s).
	/// This is expected to be called periodically during runtime.
	async fn update(&self) -> Result<()> {
		let latest_patches = self
			.thaliak
			.latest_versions(self.repositories.clone())
			.await?;

		let key = VersionKey::from_latest_patches(&latest_patches);

		// Build a mapping of patch lists keyed by repository name.
		// TODO: I'm not happy about zipping this twice, but it's so much cleaner to write than the absolute jank required by doing it all in one go.
		let pending_patch_lists = latest_patches
			.iter()
			.zip(&self.repositories)
			.map(|(patch, repository)| self.thaliak.patch_list(repository.clone(), patch));
		let patch_lists = try_join_all(pending_patch_lists).await?;
		let version_patches = self
			.repositories
			.clone()
			.into_iter()
			.zip(patch_lists)
			.collect::<HashMap<_, _>>();

		let mut versions = self.versions.write().expect("poisoned");
		let version = versions.entry(key).or_insert_with(|| Version::new());
		version.update(version_patches);
		drop(versions);

		// TODO: Update the LATEST sigil

		// TODO: persist

		self.broadcast_version_list();

		Ok(())
	}

	/// Broadcast the current list of versions to any listeners on the channel.
	fn broadcast_version_list(&self) {
		let versions = self.versions.read().expect("poisoned");
		let keys = versions.keys().cloned().collect::<Vec<_>>();
		self.channel.send_if_modified(|value| {
			if &keys != value {
				*value = keys;
				return true;
			}
			false
		});
	}
}

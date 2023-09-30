use std::{
	collections::{BTreeMap, HashMap},
	fs,
	sync::RwLock,
};

use anyhow::{Context, Ok, Result};
use figment::value::magic::RelativePathBuf;
use futures::future::try_join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{key::VersionKey, patch::Patch, persist::JsonFile, thaliak};

pub type PatchList = Vec<(String, Vec<Patch>)>;

#[derive(Debug, Deserialize)]
pub struct Config {
	thaliak: thaliak::Config,

	// interval
	directory: RelativePathBuf,
	repositories: Vec<String>,
}

#[derive(Debug)]
pub struct Manager {
	thaliak: thaliak::Provider,

	file: JsonFile,

	channel: watch::Sender<Vec<VersionKey>>,

	repositories: Vec<String>,
	// TODO: Consider using PatchList in-memory - while the map is good on disk, i'm basically just mapping to a vector format constantly at runtime
	versions: RwLock<HashMap<VersionKey, PatchList>>,
}

impl Manager {
	pub fn new(config: Config) -> Result<Self> {
		let directory = config.directory.relative();
		fs::create_dir_all(&directory)?;

		let (sender, _receiver) = watch::channel(vec![]);

		Ok(Self {
			thaliak: thaliak::Provider::new(config.thaliak),

			file: JsonFile::new(directory.join("versions.json")),

			channel: sender,

			repositories: config.repositories,
			versions: Default::default(),
		})
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

		Ok(version.clone())
	}

	/// Start the service.
	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		self.hydrate().await?;

		// TODO: timer and all that shmuck
		self.update().await?;

		Ok(())
	}

	/// Hydrate version list from local configuration.
	async fn hydrate(&self) -> Result<()> {
		let persisted_versions = self.file.read::<HashMap<VersionKey, PersitedVersion>>()?;

		let pending_hydrated_versions =
			persisted_versions
				.into_iter()
				.map(|(key, version)| async move {
					// TODO: A version made on repository list [a, b] will create a patch list [a] on an updated repository list of [a, X, b]. I'm not convinced that's a problem.
					let latest_patches = self.repositories.iter().map_while(|repository_name| {
						version
							.patches
							.get(repository_name)
							.map(|patch| (repository_name.clone(), patch))
					});
					Ok((key, self.build_patch_list(latest_patches).await?))
				});
		let hydrated_versions = try_join_all(pending_hydrated_versions).await?;

		let mut versions = self.versions.write().expect("poisoned");
		for (key, patch_list) in hydrated_versions {
			// TODO: save out names.
			versions.insert(key, patch_list);
		}
		drop(versions);

		self.broadcast_version_list();

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

		let patch_list = self
			.build_patch_list(self.repositories.iter().cloned().zip(latest_patches))
			.await?;

		let mut versions = self.versions.write().expect("poisoned");
		versions.insert(key, patch_list);
		drop(versions);

		// TODO: Update the LATEST sigil

		self.save()?;

		self.broadcast_version_list();

		Ok(())
	}

	async fn build_patch_list(
		&self,
		latest_patches: impl IntoIterator<Item = (String, impl AsRef<str>)>,
	) -> Result<PatchList> {
		let pending_patch_list =
			latest_patches
				.into_iter()
				.map(|(repository_name, latest_patch)| async move {
					Ok((
						repository_name.clone(),
						self.thaliak
							.patch_list(repository_name, latest_patch.as_ref())
							.await?,
					))
				});

		let patch_list = try_join_all(pending_patch_list).await?;

		Ok(patch_list)
	}

	fn save(&self) -> Result<()> {
		let versions = self.versions.read().expect("poisoned");

		// TODO: This is atrocious.
		let persisted_versions = versions
			.iter()
			.map(|(key, patch_list)| {
				Ok((
					key.clone(),
					PersitedVersion {
						names: vec![],
						patches: patch_list
							.iter()
							.map(|(repository, patches)| {
								Ok((
									repository.clone(),
									patches
										.last()
										.with_context(|| {
											format!("missing patches for repository {repository} in version {key}")
										})?
										.name
										.clone(),
								))
							})
							.collect::<Result<_>>()?,
					},
				))
			})
			.collect::<Result<BTreeMap<_, _>>>()?;

		self.file.write(&persisted_versions)?;

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

#[derive(Debug, Serialize, Deserialize)]
struct PersitedVersion {
	names: Vec<String>,
	patches: BTreeMap<String, String>,
}

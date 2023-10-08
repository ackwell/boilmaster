use std::{
	collections::{BTreeMap, HashMap},
	fs,
	sync::RwLock,
};

use anyhow::{Context, Result};
use either::Either;
use figment::value::magic::RelativePathBuf;
use futures::future::try_join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{
	key::VersionKey,
	persist::JsonFile,
	thaliak,
	version::{PatchList, Status, Version},
};

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
	versions: RwLock<HashMap<VersionKey, Version>>,
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

	// Get a list of all currently known versions.
	pub fn versions(&self) -> Vec<VersionKey> {
		self.versions
			.read()
			.expect("poisoned")
			.keys()
			.copied()
			.collect()
	}

	/// Resolve a version name to its key. If no version is specified, the version marked as latest will be returned, if any exists.
	pub fn resolve(&self, name: Option<&str>) -> Option<VersionKey> {
		todo!("resolve {name:?}")
	}

	pub fn version(&self, key: VersionKey) -> Result<Version> {
		let versions = self.versions.read().expect("poisoned");
		versions
			.get(&key)
			.cloned()
			.with_context(|| format!("unknown version {key}"))
	}

	/// Get a patch list for a given version.
	pub fn patch_list(&self, key: VersionKey) -> Result<PatchList> {
		let version = self.version(key)?;

		let patch_list = match version {
			Version::Available(patch_list) => patch_list,
			Version::Unavailable(..) => anyhow::bail!("unavailable version {key}"),
		};

		Ok(patch_list)
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
					anyhow::Ok((key, self.build_version(latest_patches).await?))
				});
		let hydrated_versions = try_join_all(pending_hydrated_versions).await?;

		let mut versions = self.versions.write().expect("poisoned");
		for (key, version) in hydrated_versions {
			// TODO: save out names.
			versions.insert(key, version);
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

		// Build a mapping of versions keyed by repository name.
		let patch_list = self
			.build_version(self.repositories.iter().cloned().zip(latest_patches))
			.await?;

		let mut versions = self.versions.write().expect("poisoned");
		versions.insert(key, patch_list);
		drop(versions);

		// TODO: Update the LATEST sigil

		self.save()?;

		self.broadcast_version_list();

		Ok(())
	}

	/// Build a version as defined by its latest patches
	async fn build_version(
		&self,
		latest_patches: impl IntoIterator<Item = (String, impl AsRef<str>)>,
	) -> Result<Version> {
		let pending_results =
			latest_patches
				.into_iter()
				.map(|(repository_name, latest_patch)| async move {
					let patch_list = self
						.thaliak
						.patch_list(repository_name.clone(), latest_patch.as_ref())
						.await;

					type TE = thaliak::Error;
					match patch_list {
						// Successfully resolved patch list.
						Ok(patch_list) => Ok(Either::Left((repository_name, patch_list))),

						// Couldn't resolve it - this is not a failure, we'll handle it later.
						Err(TE::UnresolvablePatch(reason)) => {
							Ok(Either::Right((repository_name, latest_patch, reason)))
						}

						// Unrecoverable error, bubble it out.
						Err(error @ TE::Failure(..)) => Err(error),
					}
				});

		let results = try_join_all(pending_results).await?;

		// TODO: work out if there's a better way of doing this - but realistically a few extra iterations over a, what? 5 item array? who cares.
		// Check if any of the patches couldn't be resolved.
		if results
			.iter()
			.any(|result| matches!(result, Either::Right(..)))
		{
			// Unresolvable version detected, build out the information for one of those.
			let patch_statuses = results
				.into_iter()
				.map(|result| {
					Ok(match result {
						Either::Left((repo, patch_list)) => (
							repo,
							// I do this call chain twice now - worth moving somewhere?
							patch_list
								.last()
								.with_context(|| format!("TODO: missing patch count"))?
								.name
								.clone(),
							Status::Ok,
						),

						Either::Right((repo, latest, reason)) => (
							repo,
							latest.as_ref().to_string(),
							Status::Unresolved(reason),
						),
					})
				})
				.collect::<Result<Vec<_>>>()?;

			return Ok(Version::Unavailable(patch_statuses));
		}

		// All the patches were resolved, build an available version
		// TODO: Probably worth saving current state to disk so that if thaliak fails we can try to restore local state before bailing entirely.
		let patch_list = results
			.into_iter()
			.map(|result| match result {
				Either::Left(patch_list) => patch_list,
				Either::Right(..) => {
					unreachable!(
						"any results with a Right entry should be captured by condition above"
					)
				}
			})
			.collect::<Vec<_>>();

		Ok(Version::Available(patch_list))
	}

	fn save(&self) -> Result<()> {
		let versions = self.versions.read().expect("poisoned");

		// TODO: This is atrocious.
		let persisted_versions = versions
			.iter()
			.map(|(key, version)| {
				Ok((
					key.clone(),
					PersitedVersion {
						names: vec![],
						patches: match version {
							Version::Available(patch_list) => patch_list
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

							Version::Unavailable(patches) => patches
								.iter()
								.map(|(repository, patch, _status)| {
									(repository.clone(), patch.clone())
								})
								.collect(),
						},
					},
				))
			})
			.collect::<Result<BTreeMap<_, _>>>()?;

		self.file.write(&persisted_versions)?;

		Ok(())
	}

	/// Broadcast the current list of versions to any listeners on the channel.
	fn broadcast_version_list(&self) {
		let keys = self.versions();
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

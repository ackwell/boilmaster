use std::collections::HashSet;

use anyhow::Result;
use serde::Deserialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{key::VersionKey, patch::Patch, thaliak};

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

	repositories: Vec<String>,
	// TODO: this should be a map of key->v
	versions: HashSet<VersionKey>,
}

impl Manager {
	pub fn new(config: Config) -> Result<Self> {
		let manager = Self {
			thaliak: thaliak::Provider::new(config.thaliak),

			repositories: config.repositories,
			versions: Default::default(),
		};

		Ok(manager)
	}

	/// Subscribe to changes to the version list.
	pub fn subscribe(&self) -> watch::Receiver<Vec<VersionKey>> {
		todo!()
	}

	/// Resolve a version name to its key. If no version is specified, the version marked as latest will be returned, if any exists.
	pub fn resolve(&self, name: Option<&str>) -> Option<VersionKey> {
		todo!()
	}

	/// Get a patch list for a given version.
	pub fn patch_list(&self, key: VersionKey) -> Result<PatchList> {
		todo!()
	}

	/// Start the service.
	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		// TODO: timer and all that shmuck
		self.update().await?;

		todo!("yahoo")
	}

	async fn update(&self) -> Result<()> {
		let foo = self
			.thaliak
			.latest_versions(self.repositories.clone())
			.await?;

		tracing::info!("LATEST: {foo:#?}");

		Ok(())
	}
}

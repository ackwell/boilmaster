use std::{
	collections::{hash_map::Entry, HashMap},
	fs,
	path::PathBuf,
	sync::{Arc, RwLock},
};

use figment::value::magic::RelativePathBuf;
use futures::future::try_join_all;
use ironworks::excel::Sheet;
use serde::Deserialize;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{search::error::Result, version::VersionKey};

use super::database::Database;

#[derive(Debug, Deserialize)]
pub struct Config {
	directory: RelativePathBuf,
	max_batch_size: usize,
}

pub struct Provider {
	directory: PathBuf,
	max_batch_size: usize,

	databases: RwLock<HashMap<VersionKey, Arc<Database>>>,
}

impl Provider {
	pub fn new(config: Config) -> Result<Self> {
		let directory = config.directory.relative();

		// ensure shit exists ig
		fs::create_dir_all(&directory)?;

		Ok(Self {
			directory,
			max_batch_size: config.max_batch_size,
			databases: Default::default(),
		})
	}

	pub async fn ingest(
		self: Arc<Self>,
		cancel: CancellationToken,
		sheets: Vec<(VersionKey, Sheet<'static, String>)>,
	) -> Result<()> {
		// Group by database key and run per-DB ingestions concurrently. Realistically
		// Sqlite doesn't support multiple writers on a single DB, but that's left as
		// an implementation detail of the DB.
		let mut grouped = HashMap::<VersionKey, Vec<Sheet<String>>>::new();
		for (version, sheet) in sheets {
			grouped.entry(version).or_insert_with(Vec::new).push(sheet);
		}

		let pending_ingestions = grouped
			.into_iter()
			.map(|(version, sheets)| self.ingest_version(version, sheets));

		select! {
			_ = cancel.cancelled() => { }
			result = try_join_all(pending_ingestions) => { result?; }
		}

		Ok(())
	}

	async fn ingest_version(
		&self,
		version: VersionKey,
		sheets: Vec<Sheet<'static, String>>,
	) -> Result<()> {
		let span = tracing::info_span!("ingest", %version);
		let database = self.database(version);
		tokio::task::spawn(async move { database.ingest(sheets).await }.instrument(span)).await?
	}

	fn database(&self, version: VersionKey) -> Arc<Database> {
		let mut write_handle = self.databases.write().expect("poisoned");
		let database = match write_handle.entry(version) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => {
				// todo log?
				let database = Database::new(
					&self.directory.join(format!("version-{version}")),
					self.max_batch_size,
				);
				entry.insert(Arc::new(database))
			}
		};

		Arc::clone(database)
	}
}

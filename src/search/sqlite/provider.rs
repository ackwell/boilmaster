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
use tokio::{select, task};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use uuid::Uuid;

use crate::{
	data::Data,
	search::{
		error::{Error, Result},
		internal_query::post,
		search::SearchResult,
	},
	version::VersionKey,
};

use super::{cursor, database::Database};

#[derive(Debug, Deserialize)]
pub struct Config {
	directory: RelativePathBuf,
	cursor: cursor::Config,
}

#[derive(Debug)]
pub enum SearchRequest {
	Query {
		version: VersionKey,
		queries: Vec<(String, post::Node)>,
	},
	Cursor(Uuid),
}

pub struct Provider {
	data: Arc<Data>,

	directory: PathBuf,

	databases: RwLock<HashMap<VersionKey, Arc<Database>>>,
	cursors: cursor::Cache,
}

impl Provider {
	pub fn new(config: Config, data: Arc<Data>) -> Result<Self> {
		let directory = config.directory.relative();
		fs::create_dir_all(&directory)?;

		Ok(Self {
			data,
			directory,
			databases: Default::default(),
			cursors: cursor::Cache::new(config.cursor),
		})
	}

	pub async fn ingest(
		self: Arc<Self>,
		cancel: CancellationToken,
		sheets: Vec<(VersionKey, Sheet<String>)>,
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
			.map(|(version, sheets)| self.ingest_version(cancel.clone(), version, sheets));

		select! {
			_ = cancel.cancelled() => { }
			result = try_join_all(pending_ingestions) => { result?; }
		}

		Ok(())
	}

	async fn ingest_version(
		&self,
		cancel: CancellationToken,
		version: VersionKey,
		sheets: Vec<Sheet<String>>,
	) -> Result<()> {
		let span = tracing::info_span!("ingest", %version);
		let database = self.database(version)?;
		task::spawn(async move { database.ingest(cancel, sheets).await }.instrument(span)).await?
	}

	pub async fn search(
		&self,
		request: SearchRequest,
		limit: usize,
	) -> Result<(Vec<SearchResult>, Option<Uuid>)> {
		let (version, database, cursor) = match request {
			SearchRequest::Query { version, queries } => {
				let database = self.database(version)?;
				let cursor = database.build_cursor(queries)?;

				(version, database, cursor)
			}

			SearchRequest::Cursor(uuid) => {
				let cursor = self
					.cursors
					.get(uuid)
					.ok_or_else(|| Error::UnknownCursor(uuid))?;

				let database = self.database(cursor.version)?;
				(cursor.version, database, cursor.inner)
			}
		};

		let (results, next_cursor) = database.search(cursor, limit).await?;

		let cursor_key =
			next_cursor.map(|inner| self.cursors.insert(cursor::Cursor { version, inner }));

		Ok((results, cursor_key))
	}

	fn database(&self, version: VersionKey) -> Result<Arc<Database>> {
		let mut write_handle = self.databases.write().expect("poisoned");
		let database = match write_handle.entry(version) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => {
				// TODO: log?
				let excel = self.data.version(version)?.excel();
				let database =
					Database::new(self.directory.join(format!("version-{version}")), excel);
				entry.insert(Arc::new(database))
			}
		};

		Ok(Arc::clone(database))
	}
}

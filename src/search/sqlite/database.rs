use std::{
	path::PathBuf,
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
};

use anyhow::anyhow;
use bb8::{Pool, PooledConnection};
use ironworks::excel::{Excel, Sheet};
use itertools::Itertools;
use sea_query::{Iden, Quote, SqliteQueryBuilder};
use sea_query_rusqlite::RusqliteBinder;
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::{
	read::LanguageString,
	search::{
		error::{Error, Result},
		internal_query::post,
		search::SearchResult,
	},
};

use super::{connection::SqliteConnectionManager, query::resolve_queries, schema::table_name};

pub struct Database {
	pool: Pool<SqliteConnectionManager>,

	ready: AtomicBool,
}

impl Database {
	pub fn new(path: PathBuf, excel: Arc<Excel>) -> Self {
		let manager = SqliteConnectionManager::new(path, excel);

		// TODO: should probably configure this a bit. stuff like a min idle of 1, etc. likely should be in config file
		let pool = Pool::builder().build_unchecked(manager);

		Self {
			pool,
			ready: false.into(),
		}
	}

	pub async fn ingest(
		&self,
		cancel: CancellationToken,
		sheets: Vec<Sheet<String>>,
	) -> Result<()> {
		// No need to re-ingest after initial stand-up.
		if self.ready.load(Ordering::Relaxed) {
			return Ok(());
		}

		let connection = self.pool.get_owned().await?;
		let task = task::spawn_blocking(move || Self::prepare(cancel, connection, sheets));
		task.await??;

		self.ready.store(true, Ordering::Relaxed);

		Ok(())
	}

	fn prepare(
		cancel: CancellationToken,
		connection: PooledConnection<SqliteConnectionManager>,
		sheets: Vec<Sheet<String>>,
	) -> Result<()> {
		tracing::debug!("preparing search database");

		for sheet in sheets {
			// If we've been asked to cancel, do so.
			if cancel.is_cancelled() {
				return Err(anyhow!("cancelling out of search database preparation").into());
			}

			let name = sheet.name();
			let languages = sheet.languages()?;
			let tables = languages
				.into_iter()
				.map(|language| {
					let table_name = table_name(&name, language).quoted(Quote::new(b'"'));
					let language_string = LanguageString::from(language);
					format!(
						r#"CREATE VIRTUAL TABLE IF NOT EXISTS "{table_name}" USING ironworks(sheet={name}, language={language_string});"#
					)
				})
				.join("\n");
			connection.execute_batch(&format!("BEGIN;\n{tables}\nCOMMIT;"))?;
		}

		tracing::debug!("search database ready");

		Ok(())
	}

	pub async fn search(&self, queries: Vec<(String, post::Node)>) -> Result<Vec<SearchResult>> {
		// Shoo off search requests when we're not ready yet.
		if !self.ready.load(Ordering::Relaxed) {
			return Err(Error::NotReady);
		}

		let statement_builder = resolve_queries(queries);
		let (query, values) = statement_builder.build_rusqlite(SqliteQueryBuilder);

		let connection = self.pool.get().await?;
		let mut statement = connection.prepare(&query)?;
		// TODO: not a fan of this implicit structure shared between query and here
		let search_results = statement
			.query_map(&*values.as_params(), |row| {
				Ok(SearchResult {
					sheet: row.get(0)?,
					row_id: row.get(1)?,
					subrow_id: row.get(2)?,
					score: row.get(3)?,
				})
			})?
			.collect::<Result<_, _>>()?;

		Ok(search_results)
	}
}

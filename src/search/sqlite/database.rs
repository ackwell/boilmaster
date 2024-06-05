use std::{cmp, path::Path, sync::OnceLock};

use ironworks::excel::{Language, Sheet};
use sea_query::SqliteQueryBuilder;
use sea_query_binder::SqlxBinder;
use sqlx::{
	sqlite::{SqliteConnectOptions, SqliteSynchronous},
	SqlitePool,
};

use crate::search::{
	error::Result,
	sqlite::query::{row_values, table_insert},
};

use super::query;

fn max_values() -> usize {
	static MAX_VALUES: OnceLock<usize> = OnceLock::new();
	*MAX_VALUES.get_or_init(|| env!("BM_SQLITE_MAX_VALUES").parse::<usize>().unwrap())
}

pub struct Database {
	max_batch_size: usize,

	pool: SqlitePool,
}

impl Database {
	pub fn new(path: &Path, max_batch_size: usize) -> Self {
		let options = SqliteConnectOptions::new()
			.filename(path)
			.create_if_missing(true)
			.synchronous(SqliteSynchronous::Off);

		let pool = SqlitePool::connect_lazy_with(options);

		Self {
			pool,
			max_batch_size,
		}
	}

	pub async fn ingest(&self, sheets: Vec<Sheet<'static, String>>) -> Result<()> {
		// NOTE: There's little point in trying to parallelise this, as sqlite only supports one writer at a time.
		// TODO: WAL mode may impact this - r&d required.
		// TODO: this should skip already-complete sheets
		for sheet in sheets {
			self.ingest_sheet(sheet).await?;
		}

		Ok(())
	}

	async fn ingest_sheet(&self, sheet: Sheet<'static, String>) -> Result<()> {
		let name = sheet.name();

		// TODO: Check if table already exists, can exit early if it does
		//       ^ needs to account for partial completion - do i store a seperate ingestion state table, or derive it from inserted data?

		tracing::debug!(sheet = name, "ingesting");

		// TODO: drop existing table (is this needed?)

		// Create table for the sheet.
		let query = query::table_create(&sheet)?.build(SqliteQueryBuilder);
		sqlx::query(&query).execute(&self.pool).await?;

		// Insert the data.
		let base_statement = table_insert(&sheet)?;
		let columns = sheet.columns()?;

		let batch_size = cmp::min(max_values() / columns.len(), self.max_batch_size);

		// NOTE: This mess because itertools' chunk isn't Send.
		let mut count = 0;
		let mut statement = base_statement.clone();

		// TODO: I _need_ to handle languages. How? do i split columns or tables? columns would risk blow out pretty quickly. game uses tables.hm.
		for row in sheet.with().language(Language::English).iter() {
			let values = row_values(&sheet, &row, columns.iter())?;
			statement.values_panic(values);

			count += 1;
			if count >= batch_size {
				count = 0;

				let (query, values) = statement.build_sqlx(SqliteQueryBuilder);
				sqlx::query_with(&query, values).execute(&self.pool).await?;

				statement = base_statement.clone();
			}
		}

		if count > 0 {
			let (query, values) = statement.build_sqlx(SqliteQueryBuilder);
			sqlx::query_with(&query, values).execute(&self.pool).await?;
		}

		Ok(())
	}
}

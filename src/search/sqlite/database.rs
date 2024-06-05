use std::path::Path;

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

pub struct Database {
	pool: SqlitePool,
}

impl Database {
	pub fn new(path: &Path) -> Self {
		let options = SqliteConnectOptions::new()
			.filename(path)
			.create_if_missing(true)
			.synchronous(SqliteSynchronous::Off);

		let pool = SqlitePool::connect_lazy_with(options);

		Self { pool }
	}

	pub async fn ingest(&self, sheets: Vec<Sheet<'static, String>>) -> Result<()> {
		// NOTE: There's little point in trying to parallelise this, as sqlite only supports one writer at a time.
		// TODO: WAL mode may impact this - r&d required.
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

		// TODO: insert
		//       work out what i can get away with for batching
		let base_query = table_insert(&sheet)?;
		let columns = sheet.columns()?;

		// TODO: I _need_ to handle languages. How? do i split columns or tables? columns would risk blow out pretty quickly. game uses tables.hm.
		for row in sheet.with().language(Language::English).iter() {
			let values = row_values(&sheet, &row, columns.iter())?;
			let (query, sqlx_values) = base_query
				.clone()
				.values_panic(values)
				.build_sqlx(SqliteQueryBuilder);
			sqlx::query_with(&query, sqlx_values)
				.execute(&self.pool)
				.await?;
		}

		Ok(())
	}
}

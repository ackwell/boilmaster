use std::{cmp, collections::HashSet, path::Path, sync::OnceLock};

use ironworks::excel::{Language, Sheet};
use sea_query::{ColumnDef, Expr, Iden, Query, SqliteQueryBuilder, Table};
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

#[derive(Iden)]
enum Metadata {
	Table,
	Sheet,
	Ingested,
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
		let completed_sheets = self.completed_sheets().await?;
		let mut skipped = 0;

		// NOTE: There's little point in trying to parallelise this, as sqlite only supports one writer at a time.
		// TODO: WAL mode may impact this - r&d required.
		for sheet in &sheets {
			if completed_sheets.contains(&sheet.name()) {
				skipped += 1;
				continue;
			}

			for language in sheet.languages()? {
				self.ingest_sheet(sheet, language).await?;
			}

			self.mark_completed(sheet).await?;
		}

		if sheets.len() - skipped > 0 {
			tracing::info!("ingestion complete");

			if skipped > 0 {
				tracing::debug!("skipped {skipped} already-ingested sheets");
			}
		}

		Ok(())
	}

	async fn ingest_sheet(&self, sheet: &Sheet<'static, String>, language: Language) -> Result<()> {
		tracing::debug!(sheet = sheet.name(), ?language, "ingesting");

		// Drop any existing table by this name. May occur if a prior instance was interrupted during ingestion.
		let query = query::table_drop(sheet, language).build(SqliteQueryBuilder);
		sqlx::query(&query).execute(&self.pool).await?;

		// Create table for the sheet.
		let query = query::table_create(sheet, language)?.build(SqliteQueryBuilder);
		sqlx::query(&query).execute(&self.pool).await?;

		// Insert the data.
		let base_statement = table_insert(sheet, language)?;
		let columns = sheet.columns()?;

		let batch_size = cmp::min(max_values() / columns.len(), self.max_batch_size);

		// NOTE: This mess because itertools' chunk isn't Send.
		let mut count = 0;
		let mut statement = base_statement.clone();

		for row in sheet.with().language(language).iter() {
			let values = row_values(sheet, &row, columns.iter())?;
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

	async fn completed_sheets(&self) -> Result<HashSet<String>> {
		// Ensure meta exists.
		let query = Table::create()
			.table(Metadata::Table)
			.if_not_exists()
			.col(ColumnDef::new(Metadata::Sheet).text().primary_key())
			.col(ColumnDef::new(Metadata::Ingested).boolean())
			.to_string(SqliteQueryBuilder);
		sqlx::query(&query).execute(&self.pool).await?;

		// Get list of sheets marked as ingested.
		let query = Query::select()
			.column(Metadata::Sheet)
			.from(Metadata::Table)
			.and_where(Expr::col(Metadata::Ingested).is(true))
			.to_string(SqliteQueryBuilder);
		let results: Vec<(String,)> = sqlx::query_as(&query).fetch_all(&self.pool).await?;

		Ok(results.into_iter().map(|(name,)| name).collect())
	}

	async fn mark_completed(&self, sheet: &Sheet<'_, String>) -> Result<()> {
		let (query, values) = Query::insert()
			.into_table(Metadata::Table)
			.columns([Metadata::Sheet, Metadata::Ingested])
			.values_panic([sheet.name().into(), true.into()])
			.build_sqlx(SqliteQueryBuilder);
		sqlx::query_with(&query, values).execute(&self.pool).await?;

		Ok(())
	}
}

use std::{
	collections::{hash_map::Entry, HashMap},
	fs,
	path::{Path, PathBuf},
	sync::{Arc, RwLock},
};

use figment::value::magic::RelativePathBuf;
use futures::future::try_join_all;
use ironworks::{
	excel::{Field, Language, Sheet},
	file::exh,
};
use sea_query::{
	Alias, ColumnDef, ColumnType, Query, SimpleExpr, SqliteQueryBuilder, Table,
	TableCreateStatement,
};
use sea_query_binder::SqlxBinder;
use serde::Deserialize;
use sqlx::{
	sqlite::{SqliteConnectOptions, SqliteSynchronous},
	SqlitePool,
};
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{search::error::Result, version::VersionKey};

#[derive(Debug, Deserialize)]
pub struct Config {
	directory: RelativePathBuf,
}

pub struct Provider {
	directory: PathBuf,

	databases: RwLock<HashMap<VersionKey, Arc<Database>>>,
}

impl Provider {
	pub fn new(config: Config) -> Result<Self> {
		let directory = config.directory.relative();

		// ensure shit exists ig
		fs::create_dir_all(&directory)?;

		Ok(Self {
			directory,
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
			_ = cancel.cancelled() => { todo!() }
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
				let database = Database::new(&self.directory.join(format!("version-{version}")));
				entry.insert(Arc::new(database))
			}
		};

		Arc::clone(database)
	}
}

struct Database {
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
		// TODO: This is suspciously similar to create_table shit. inline?
		let name = sheet.name();
		let kind = sheet.kind()?;

		// TODO: Check if table already exists, can exit early if it does
		//       ^ needs to account for partial completion - do i store a seperate ingestion state table, or derive it from inserted data?

		tracing::debug!(sheet = name, "ingesting");

		// TODO: drop existing table (is this needed?)

		// Create table for the sheet.
		let query = create_table(&sheet)?.build(SqliteQueryBuilder);
		sqlx::query(&query).execute(&self.pool).await?;

		// TODO: insert
		//       work out what i can get away with for batching

		// TODO: should I reuse the idens for stuff like table name within this code path?
		let sheet_columns = sheet.columns()?;
		let mut table_columns = vec![Alias::new("row_id")];
		if matches!(kind, exh::SheetKind::Subrows) {
			table_columns.push(Alias::new("subrow_id"))
		}
		for column in &sheet_columns {
			table_columns.push(column_name(column))
		}
		let base_query = Query::insert()
			.into_table(Alias::new(name))
			.columns(table_columns)
			.to_owned();

		// TODO: I _need_ to handle languages. How? do i split columns or tables? columns would risk blow out pretty quickly. game uses tables.hm.
		for row in sheet.with().language(Language::English).iter() {
			//
			let mut values: Vec<SimpleExpr> = vec![row.row_id().into()];
			if matches!(kind, exh::SheetKind::Subrows) {
				values.push(row.subrow_id().into())
			}
			for column in &sheet_columns {
				let field = row.field(column)?;
				use Field as F;
				values.push(match field {
					F::String(sestring) => sestring.to_string().into(),
					F::Bool(value) => value.into(),
					F::I8(value) => value.into(),
					F::I16(value) => value.into(),
					F::I32(value) => value.into(),
					F::I64(value) => value.into(),
					F::U8(value) => value.into(),
					F::U16(value) => value.into(),
					F::U32(value) => value.into(),
					F::U64(value) => value.into(),
					F::F32(value) => value.into(),
				})
			}
			let (query, values) = base_query
				.clone()
				.values_panic(values)
				.build_sqlx(SqliteQueryBuilder);
			sqlx::query_with(&query, values).execute(&self.pool).await?;
		}

		Ok(())
	}
}

fn create_table(sheet: &Sheet<String>) -> Result<TableCreateStatement> {
	let name = sheet.name();
	let kind = sheet.kind()?;

	// NOTE: Opting against a WITHOUT ROWID table for these - the benefits they
	// confer aren't particularly meaningful for our workload.
	let mut table = Table::create();
	table
		.table(Alias::new(name))
		// TODO: use well-known aliases for these?
		.col(ColumnDef::new(Alias::new("row_id")).integer().primary_key());

	if matches!(kind, exh::SheetKind::Subrows) {
		table.col(ColumnDef::new(Alias::new("subrow_id")).integer());
	}

	for column in sheet.columns()? {
		table.col(&mut column_definition(&column));
	}

	Ok(table.take())
}

fn column_definition(column: &exh::ColumnDefinition) -> ColumnDef {
	let name = column_name(column);

	use exh::ColumnKind as CK;
	let kind = match column.kind() {
		// Using text for this because we have absolutely no idea how large any given string is going to be.
		CK::String => ColumnType::Text,

		// Pretty much all of this will collapse to "INTEGER" on sqlite but hey. Accuracy.
		CK::Int8 => ColumnType::TinyInteger,
		CK::UInt8 => ColumnType::TinyUnsigned,
		CK::Int16 => ColumnType::SmallInteger,
		CK::UInt16 => ColumnType::SmallUnsigned,
		CK::Int32 => ColumnType::Integer,
		CK::UInt32 => ColumnType::Unsigned,
		CK::Int64 => ColumnType::BigInteger,
		CK::UInt64 => ColumnType::BigUnsigned,
		CK::Float32 => ColumnType::Float,

		CK::Bool
		| CK::PackedBool0
		| CK::PackedBool1
		| CK::PackedBool2
		| CK::PackedBool3
		| CK::PackedBool4
		| CK::PackedBool5
		| CK::PackedBool6
		| CK::PackedBool7 => ColumnType::Boolean,
	};

	ColumnDef::new_with_type(name, kind)
}

fn column_name(column: &exh::ColumnDefinition) -> Alias {
	let offset = column.offset();

	// For packed bool columns, offset alone is not enough to disambiguate a
	// field - add a suffix of the packed bit position.
	use exh::ColumnKind as CK;
	let suffix = match column.kind() {
		CK::PackedBool0 => "_0",
		CK::PackedBool1 => "_1",
		CK::PackedBool2 => "_2",
		CK::PackedBool3 => "_3",
		CK::PackedBool4 => "_4",
		CK::PackedBool5 => "_5",
		CK::PackedBool6 => "_6",
		CK::PackedBool7 => "_7",
		_ => "",
	};

	Alias::new(format!("{offset}{suffix}"))
}

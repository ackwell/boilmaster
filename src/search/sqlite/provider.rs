use std::{
	borrow::Cow,
	collections::{hash_map::Entry, HashMap},
	str::FromStr,
	sync::Arc,
};

use ironworks::{
	excel::{Field, Language, Row, Sheet},
	file::exh,
};
use itertools::Itertools;
use sqlx::{
	sqlite::{SqliteConnectOptions, SqliteSynchronous},
	SqlitePool,
};
use tokio::{select, sync::RwLock};
use tokio_util::sync::CancellationToken;

use crate::{search::error::Result, version::VersionKey};

pub struct Provider {
	// todo: should this instead have per-pool locks? like that other thing i did somewhen where it inserts a pending item so it doesn't lock the entire map
	dbs: RwLock<HashMap<VersionKey, Arc<SqlitePool>>>,
}

impl Provider {
	pub fn new() -> Self {
		Self {
			dbs: Default::default(),
		}
	}

	#[tracing::instrument(skip_all)]
	pub async fn ingest(
		self: Arc<Self>,
		cancel: CancellationToken,
		sheets: Vec<(VersionKey, Sheet<'static, String>)>,
	) -> Result<()> {
		// db per ver? share? duno, think about it.
		// consider WAL for journalling. possibly not worth it if using a db per version, as we'd never be writing at the same time as reading. https://www.sqlite.org/wal.html

		// TODO: consider seaquery for this shit
		tracing::info!("starting the thing ({})", sheets.len());
		for (version, sheet) in sheets {
			let f = Arc::clone(&self);
			select! {
				_ = cancel.cancelled() => { break }
				result = tokio::task::spawn(async move {
					f.clone().ingest_sheet(version,sheet).await
				}) => { result? }
			}
		}

		Ok(())
	}

	async fn ingest_sheet(self: Arc<Self>, version: VersionKey, sheet: Sheet<'static, String>) {
		let mut dbw = self.dbs.write().await;
		let pool = match dbw.entry(version) {
			Entry::Occupied(x) => x.into_mut(),
			Entry::Vacant(x) => {
				tracing::info!("yeeting new db i think for {version}");
				let fdsaaa = create_db(version).await;
				x.insert(Arc::new(fdsaaa))
			}
		};
		let pool = Arc::clone(pool);
		drop(dbw);

		// create the table
		create_table(pool.as_ref(), &sheet).await;

		// fdsafasd
		ingest_rows(pool.as_ref(), &sheet).await;
	}
}

async fn create_db(version: VersionKey) -> SqlitePool {
	let options = SqliteConnectOptions::from_str(&format!("{}.db", version.to_string()))
		.expect("what")
		.create_if_missing(true)
		.synchronous(SqliteSynchronous::Off);
	SqlitePool::connect_with(options).await.expect("todo")
}

async fn create_table(pool: &SqlitePool, sheet: &Sheet<'_, String>) {
	let kind = sheet.kind().expect("fasdf");
	let mut ids = vec!["row_id BIGINT"];
	if matches!(kind, exh::SheetKind::Subrows) {
		ids.push("subrow_id INTEGER");
	}

	// TODO: other languages - should they be different tables?
	let table_name = sheet.name();
	let columns = sheet
		.columns()
		.expect("todo")
		.into_iter()
		.map(|column| Cow::Owned(create_column(&column)));

	// todo: jsonb lol?

	let thing = ids.into_iter().map(Cow::Borrowed).chain(columns).join(",");

	sqlx::query(&format!(
		"CREATE TABLE IF NOT EXISTS \"{table_name}\" ({thing});"
	))
	.execute(pool)
	.await
	.expect("boom");

	let indexcols = match kind {
		exh::SheetKind::Subrows => "row_id, subrow_id",
		_ => "row_id",
	};
	sqlx::query(&format!(
		"CREATE UNIQUE INDEX \"{table_name}$id\" ON {table_name} ({indexcols})"
	))
	.execute(pool)
	.await
	.expect("boom");
}

fn create_column(column: &exh::ColumnDefinition) -> String {
	let name = column_field_name(&column);

	use exh::ColumnKind as CK;
	let kind = match column.kind() {
		CK::String => "TEXT",

		CK::Int8 | CK::UInt8 | CK::Int16 => "SMALLINT",
		CK::UInt16 | CK::Int32 => "INTEGER",
		CK::UInt32 | CK::Int64 => "BIGINT",
		CK::UInt64 => "NUMERIC",
		CK::Float32 => "REAL",

		CK::Bool
		| CK::PackedBool0
		| CK::PackedBool1
		| CK::PackedBool2
		| CK::PackedBool3
		| CK::PackedBool4
		| CK::PackedBool5
		| CK::PackedBool6
		| CK::PackedBool7 => "BOOLEAN",
	};

	format!("\"{name}\" {kind}")
}

async fn ingest_rows(pool: &SqlitePool, sheet: &Sheet<'_, String>) {
	let name = sheet.name();
	tracing::info!("ingesting {}", name);
	let columns = sheet.columns().expect("boop");
	let kind = sheet.kind().expect("fdsa");

	for row in sheet.with().language(Language::English).iter() {
		// TODO: work out how to use a prepared query for this
		let q = row_values(row, kind, &columns);
		let rq = format!("INSERT INTO \"{}\" VALUES ({q})", name);
		let fdsa = sqlx::query(&rq).execute(pool).await;
		if let Err(error) = fdsa {
			tracing::error!("bang {error}");
		}
	}

	// note: insert value count is bounded by max column count, which would mean >1 row would fail for charamaketype and such. if i want to chunk, i'll need to drastically bump max column count
}

fn row_values(row: Row, kind: exh::SheetKind, columns: &[exh::ColumnDefinition]) -> String {
	// todo: work out how the hell you're supposed to bind an array of this shit. sqlx has an outstanding thing for years though

	let mut ids = vec![Cow::Owned(row.row_id().to_string())];
	if matches!(kind, exh::SheetKind::Subrows) {
		ids.push(Cow::Owned(row.subrow_id().to_string()));
	}

	let eml = columns.iter().map(|column| -> Cow<str> {
		let field = row.field(column).expect("whatever");
		use Field as F;
		match field {
			F::String(sestring) => format!("'{}'", sestring.to_string().replace("'", "''")).into(),
			F::Bool(value) => (match value {
				true => "1",
				false => "0",
			})
			.into(),
			F::I8(value) => value.to_string().into(),
			F::I16(value) => value.to_string().into(),
			F::I32(value) => value.to_string().into(),
			F::I64(value) => value.to_string().into(),
			F::U8(value) => value.to_string().into(),
			F::U16(value) => value.to_string().into(),
			F::U32(value) => value.to_string().into(),
			F::U64(value) => value.to_string().into(),
			F::F32(value) => value.to_string().into(),
		}
	});

	let fuck = ids.into_iter().chain(eml).join(",");

	fuck
}

// todo: language shit?
pub fn column_field_name(column: &exh::ColumnDefinition) -> String {
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

	let offset = column.offset();
	format!("{offset}{suffix}")
}

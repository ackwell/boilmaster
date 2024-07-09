use std::{marker::PhantomData, os::raw::c_int, sync::Arc};

use ironworks::{excel, file::exh};
use rusqlite::{types::ToSqlOutput, vtab, Connection, ToSql};
use sea_query::{Alias, ColumnDef, SqliteQueryBuilder, Table};

use crate::{
	read::LanguageString,
	search::sqlite::schema::{column_name, column_type},
};

use super::schema::KnownColumn;

pub fn load_module(connection: &Connection, excel: Arc<excel::Excel>) -> rusqlite::Result<()> {
	let module = vtab::read_only_module::<'_, IronworksTable>();
	connection.create_module("ironworks", module, Some(excel))
}

#[derive(Debug)]
#[repr(C)]
struct IronworksTable {
	base: vtab::sqlite3_vtab,

	excel: Arc<excel::Excel>,
	sheet: String,
	language: excel::Language,
}

unsafe impl<'vtab> vtab::VTab<'vtab> for IronworksTable {
	type Aux = Arc<excel::Excel>;

	type Cursor = IronworksTableCursor<'vtab>;

	fn connect(
		db: &mut vtab::VTabConnection,
		aux: Option<&Self::Aux>,
		args: &[&[u8]],
	) -> rusqlite::Result<(String, Self)> {
		// This should never occur, but sanity check.
		let excel = aux
			.ok_or_else(|| module_error("vtable connection missing aux excel instance"))?
			.clone();

		// Parse arguments - first few are standard, rest come from table declaration.
		if args.len() < 4 {
			return Err(module_error("no arguments specified"));
		}

		let mut vtable = Self {
			base: Default::default(),
			excel,
			sheet: "".into(),
			language: excel::Language::None,
		};

		for slice in &args[3..] {
			let (key, value) = vtab::parameter(slice)?;
			match key {
				"sheet" => {
					let list = vtable.excel.list().map_err(module_error)?;

					if !list.has(value) {
						return Err(module_error(format!("unknown sheet {value}")));
					}

					vtable.sheet = value.into();
				}

				"language" => {
					let language_string = value.parse::<LanguageString>().map_err(module_error)?;
					vtable.language = language_string.into();
				}

				other => {
					return Err(module_error(format!("unknown parameter {other}")));
				}
			}
		}

		if vtable.sheet.is_empty() {
			return Err(module_error("no sheet specified"));
		}

		// Build the virtual table schema.
		let sheet_data = vtable.excel.sheet(&vtable.sheet).map_err(module_error)?;

		// NOTE: Table name is ignored in virtual table schema declarations.
		let mut table = Table::create();
		table
			.table(Alias::new("x"))
			.col(ColumnDef::new(KnownColumn::RowId).integer())
			.col(ColumnDef::new(KnownColumn::SubrowId).integer());

		for column in sheet_data.columns().map_err(module_error)? {
			table.col(&mut ColumnDef::new_with_type(
				column_name(&column),
				column_type(&column),
			));
		}

		let schema = table.build(SqliteQueryBuilder);

		db.config(vtab::VTabConfig::DirectOnly)?;

		Ok((schema, vtable))
	}

	fn best_index(&self, info: &mut vtab::IndexInfo) -> rusqlite::Result<()> {
		let mut use_row_id = false;
		for (constraint, mut usage) in info.constraints_and_usages() {
			// Optimisation: If any of the constraints include an EQ targeting a row_id, we can skip scanning the table.
			// TODO: If i add ROWID support to this, it's also applicable for this lookup.
			if constraint.is_usable()
				&& constraint.column() == 0
				&& constraint.operator() == vtab::IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
			{
				use_row_id = true;
				usage.set_argv_index(1);
			}
		}

		match use_row_id {
			true => {
				info.set_idx_num(Index::ROW_ID);
				info.set_estimated_cost(1_f64);
			}
			false => {
				info.set_idx_num(Index::SCAN);
				info.set_estimated_cost(1000000_f64);
			}
		}

		Ok(())
	}

	fn open(&'vtab mut self) -> rusqlite::Result<Self::Cursor> {
		Ok(IronworksTableCursor::new(
			self.excel.clone(),
			self.sheet.clone(),
			self.language,
		))
	}
}

impl<'vtab> vtab::CreateVTab<'vtab> for IronworksTable {
	const KIND: vtab::VTabKind = vtab::VTabKind::Default;
}

#[derive(Debug)]
enum Index {
	Scan(excel::SheetIterator<String>),
	RowId(Option<excel::Row>),
}

impl Index {
	const SCAN: c_int = 0;
	const ROW_ID: c_int = 1;
}

impl Iterator for Index {
	type Item = excel::Row;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Scan(sheet_iterator) => sheet_iterator.next(),
			Self::RowId(maybe_row) => maybe_row.take(),
		}
	}
}

#[derive(Debug)]
#[repr(C)]
struct IronworksTableCursor<'vtab> {
	base: vtab::sqlite3_vtab_cursor,

	excel: Arc<excel::Excel>,
	sheet: String,
	language: excel::Language,

	// TODO: this should probably be an enum so i can handle multiple index types
	state: Option<(Vec<exh::ColumnDefinition>, Index)>,
	next: Option<excel::Row>,

	phantom: PhantomData<&'vtab ()>,
}

impl IronworksTableCursor<'_> {
	fn new(excel: Arc<excel::Excel>, sheet: String, language: excel::Language) -> Self {
		Self {
			base: Default::default(),
			excel,
			sheet,
			language,
			state: None,
			next: None,
			phantom: PhantomData,
		}
	}
}

unsafe impl vtab::VTabCursor for IronworksTableCursor<'_> {
	fn filter(
		&mut self,
		index_number: c_int,
		_index_string: Option<&str>,
		arguments: &vtab::Values<'_>,
	) -> rusqlite::Result<()> {
		let sheet = self
			.excel
			.sheet(self.sheet.clone())
			.map_err(module_error)?
			.with_default_language(self.language);

		let columns = sheet.columns().map_err(module_error)?;

		let iterator = match index_number {
			Index::SCAN => Index::Scan(sheet.into_iter()),
			Index::ROW_ID => {
				let row_id = arguments.get::<u32>(0)?;
				// TODO: should this silently ->option instead?
				let row = sheet.row(row_id).map_err(module_error)?;
				Index::RowId(Some(row))
			}

			other => return Err(module_error(format!("unknown index {other}"))),
		};

		self.state = Some((columns, iterator));

		self.next()
	}

	fn next(&mut self) -> rusqlite::Result<()> {
		let Some((_columns, iterator)) = &mut self.state else {
			return Err(module_error("iterator was not initialised before next"));
		};

		self.next = iterator.next();

		Ok(())
	}

	fn eof(&self) -> bool {
		// NOTE: this assumes that, given .filter will set up the iterator and call next, and next will only set None if it's EOF, that next being none _is_ eof.
		self.next.is_none()
	}

	fn column(&self, context: &mut vtab::Context, index: c_int) -> rusqlite::Result<()> {
		let (Some(row), Some((columns, _iterator))) = (&self.next, &self.state) else {
			return Err(module_error("trying to access column at eof"));
		};

		match index {
			// First two columns are reserved for row IDs.
			0 => context.set_result(&row.row_id().to_sql()?)?,
			1 => context.set_result(&row.subrow_id().to_sql()?)?,

			// Remainder index into the sheet column list.
			other => {
				let column_index = usize::try_from(other - 2).map_err(module_error)?;
				let field = row.field(&columns[column_index]).map_err(module_error)?;
				context.set_result(&FieldToSql(field))?;
			}
		}

		Ok(())
	}

	fn rowid(&self) -> rusqlite::Result<i64> {
		todo!("rowid")
	}
}

struct FieldToSql(excel::Field);
impl ToSql for FieldToSql {
	fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
		Ok(match &self.0 {
			// Strings need to be converted from SeString to plain strings.
			excel::Field::String(value) => ToSqlOutput::Owned(value.to_string().into()),

			// SQLite doesn't support u64, it only goes up to i64. Try mapping down.
			excel::Field::U64(value) => {
				ToSqlOutput::Owned(i64::try_from(*value).map_err(module_error)?.into())
			}

			// Trivial.
			excel::Field::Bool(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::I8(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::I16(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::I32(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::I64(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::U8(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::U16(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::U32(value) => ToSqlOutput::Owned((*value).into()),
			excel::Field::F32(value) => ToSqlOutput::Owned((*value).into()),
		})
	}
}

fn module_error(error: impl ToString) -> rusqlite::Error {
	rusqlite::Error::ModuleError(error.to_string())
}

use std::{marker::PhantomData, sync::Arc};

use ironworks::excel;
use rusqlite::{vtab, Connection};
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
		todo!("best index")
	}

	fn open(&'vtab mut self) -> rusqlite::Result<Self::Cursor> {
		todo!("open")
	}
}

impl<'vtab> vtab::CreateVTab<'vtab> for IronworksTable {
	const KIND: vtab::VTabKind = vtab::VTabKind::Default;
}

#[derive(Debug)]
#[repr(C)]
struct IronworksTableCursor<'vtab> {
	base: vtab::sqlite3_vtab_cursor,

	phantom: PhantomData<&'vtab ()>,
}

unsafe impl vtab::VTabCursor for IronworksTableCursor<'_> {
	fn filter(
		&mut self,
		idx_num: std::os::raw::c_int,
		idx_str: Option<&str>,
		args: &vtab::Values<'_>,
	) -> rusqlite::Result<()> {
		todo!("filter")
	}

	fn next(&mut self) -> rusqlite::Result<()> {
		todo!("next")
	}

	fn eof(&self) -> bool {
		todo!("eof")
	}

	fn column(&self, ctx: &mut vtab::Context, i: std::os::raw::c_int) -> rusqlite::Result<()> {
		todo!("column")
	}

	fn rowid(&self) -> rusqlite::Result<i64> {
		todo!("rowid")
	}
}

fn module_error(error: impl ToString) -> rusqlite::Error {
	rusqlite::Error::ModuleError(error.to_string())
}

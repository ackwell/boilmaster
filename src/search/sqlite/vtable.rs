use std::marker::PhantomData;

use rusqlite::{vtab, Connection};

pub fn load_module(connection: &Connection) -> rusqlite::Result<()> {
	let module = vtab::read_only_module::<'_, IronworksTable>();
	connection.create_module("ironworks", module, None)
}

#[derive(Debug)]
#[repr(C)]
struct IronworksTable {
	base: vtab::sqlite3_vtab,
}

unsafe impl<'vtab> vtab::VTab<'vtab> for IronworksTable {
	type Aux = ();

	type Cursor = IronworksTableCursor<'vtab>;

	fn connect(
		db: &mut vtab::VTabConnection,
		aux: Option<&Self::Aux>,
		args: &[&[u8]],
	) -> rusqlite::Result<(String, Self)> {
		todo!("connect")
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

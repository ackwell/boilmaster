use std::{path::PathBuf, sync::Arc};

use bb8::ManageConnection;
use ironworks::excel::Excel;

use super::vtable;

pub struct SqliteConnectionManager {
	path: PathBuf,
	excel: Arc<Excel>,
}

impl SqliteConnectionManager {
	pub fn new(path: PathBuf, excel: Arc<Excel>) -> Self {
		Self { path, excel }
	}
}

impl ManageConnection for SqliteConnectionManager {
	type Connection = rusqlite::Connection;
	type Error = rusqlite::Error;

	async fn connect(&self) -> Result<Self::Connection, Self::Error> {
		let connection = rusqlite::Connection::open(&self.path)?;

		connection.pragma_update(None, "synchronous", "OFF")?;

		vtable::load_module(&connection, self.excel.clone())?;

		Ok(connection)
	}

	async fn is_valid(&self, connection: &mut Self::Connection) -> Result<(), Self::Error> {
		connection.execute_batch("")
	}

	fn has_broken(&self, _connection: &mut Self::Connection) -> bool {
		false
	}
}

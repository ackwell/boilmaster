use std::sync::Arc;

use async_trait::async_trait;
use bb8::ManageConnection;
use ironworks::excel::Excel;

use crate::version::VersionKey;

use super::vtable;

pub struct SqliteConnectionManager {
	version: VersionKey,
	excel: Arc<Excel<'static>>,
}

impl SqliteConnectionManager {
	pub fn new(version: VersionKey, excel: Arc<Excel<'static>>) -> Self {
		Self { version, excel }
	}
}

#[async_trait]
impl ManageConnection for SqliteConnectionManager {
	type Connection = rusqlite::Connection;
	type Error = rusqlite::Error;

	async fn connect(&self) -> Result<Self::Connection, Self::Error> {
		// TODO: should i have a configurable prefix? or just uuid it?
		let connection =
			rusqlite::Connection::open(format!("file:{}?mode=memory&cache=shared", self.version))?;

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

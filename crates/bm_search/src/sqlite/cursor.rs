use std::time::Duration;

use bm_version::VersionKey;
use mini_moka::sync as moka;
use sea_query::SelectStatement;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Cursor {
	pub version: VersionKey,
	pub inner: DatabaseCursor,
}

#[derive(Debug, Clone)]
pub struct DatabaseCursor {
	pub statement: SelectStatement,
	pub offset: usize,
}

#[derive(Debug, Deserialize)]
pub struct Config {
	ttl: Option<u64>,
	tti: Option<u64>,
}

pub struct Cache {
	cache: moka::Cache<Uuid, Cursor>,
}

impl Cache {
	pub fn new(config: Config) -> Self {
		let mut builder = moka::Cache::builder();
		if let Some(ttl) = config.ttl {
			builder = builder.time_to_live(Duration::from_secs(ttl));
		}
		if let Some(tti) = config.tti {
			builder = builder.time_to_idle(Duration::from_secs(tti));
		}

		Self {
			cache: builder.build(),
		}
	}

	pub fn get(&self, key: Uuid) -> Option<Cursor> {
		self.cache.get(&key)
	}

	pub fn insert(&self, cursor: Cursor) -> Uuid {
		let key = Uuid::new_v4();
		self.cache.insert(key, cursor);
		key
	}
}

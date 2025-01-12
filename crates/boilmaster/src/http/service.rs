use std::sync::Arc;

use crate::{asset, read, schema, search};

pub type Asset = Arc<asset::Service>;
pub type Data = Arc<bm_data::Data>;
pub type Read = Arc<read::Read>;
pub type Schema = Arc<schema::Provider>;
pub type Search = Arc<search::Search>;
pub type Version = Arc<bm_version::Manager>;

#[derive(Clone)]
pub struct Service {
	pub asset: Asset,
	pub data: Data,
	pub read: Read,
	pub schema: Schema,
	pub search: Search,
	pub version: Version,
}

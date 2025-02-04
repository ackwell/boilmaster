use std::sync::Arc;

pub type Asset = Arc<bm_asset::Service>;
pub type Data = Arc<bm_data::Data>;
pub type Read = Arc<bm_read::Read>;
pub type Schema = Arc<bm_schema::Provider>;
pub type Search = Arc<bm_search::Search>;
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

use std::sync::Arc;

use axum::extract::FromRef;

use crate::{data, schema, version};

pub type Data = Arc<data::Data>;
pub type Schema = Arc<schema::Provider>;
pub type Version = Arc<version::Manager>;

#[derive(Clone, FromRef)]
pub struct State {
	pub data: Data,
	pub schema: Schema,
	pub version: Version,
}

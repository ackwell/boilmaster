use std::sync::Arc;

use axum::extract::FromRef;

use crate::{data, version};

pub type Data = Arc<data::Data>;
pub type Version = Arc<version::Manager>;

#[derive(Clone, FromRef)]
pub struct State {
	pub data: Data,
	pub version: Version,
}

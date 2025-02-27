use axum::{Router, middleware};
use serde::Deserialize;

use crate::http::HttpState;

use super::{
	auth::{BasicAuth, basic_auth},
	version, versions,
};

#[derive(Debug, Deserialize)]
pub struct Config {
	auth: BasicAuth,
}

pub fn router(config: Config, state: HttpState) -> Router {
	Router::new()
		.merge(versions::router(state.clone()))
		.merge(version::router(state))
		.layer(middleware::from_fn_with_state(config.auth, basic_auth))
}

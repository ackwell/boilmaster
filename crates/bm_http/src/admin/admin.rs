use axum::{middleware, Router};
use serde::Deserialize;

use crate::http::HttpState;

use super::{
	auth::{basic_auth, BasicAuth},
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

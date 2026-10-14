use axum::{
	Router, debug_handler, extract::State, middleware, response::IntoResponse, routing::get,
};
use serde::Deserialize;

use crate::{http::HttpState, service::Service};

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
		.merge(version::router(state.clone()))
		.route("/thaliak2", get(test_thaliak2).with_state(state))
		.layer(middleware::from_fn_with_state(config.auth, basic_auth))
}

#[debug_handler]
async fn test_thaliak2(State(Service { version, .. }): State<Service>) -> impl IntoResponse {
	format!("{:#?}", version.test_thaliak2().await)
}

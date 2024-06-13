use std::sync::Arc;

use aide::{axum::ApiRouter, openapi::OpenApi};
use axum::{debug_handler, response::IntoResponse, routing::get, Extension, Json, Router};
use serde::Deserialize;

use crate::http::service;

use super::{asset, sheet};

#[derive(Debug, Deserialize)]
pub struct Config {
	sheet: sheet::Config,
}

pub fn router(config: Config) -> Router<service::State> {
	let mut openapi = OpenApi::default();

	ApiRouter::new()
		.nest("/sheet", sheet::router(config.sheet))
		.nest("/asset", asset::router())
		.finish_api(&mut openapi)
		.route("/openapi.json", get(openapi_json))
		.layer(Extension(Arc::new(openapi)))
}

#[debug_handler]
async fn openapi_json(Extension(openapi): Extension<Arc<OpenApi>>) -> impl IntoResponse {
	Json(&*openapi).into_response()
}

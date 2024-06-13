use std::sync::Arc;

use aide::{axum::ApiRouter, openapi, transform::TransformOpenApi};
use axum::{debug_handler, response::IntoResponse, routing::get, Extension, Json, Router};
use maud::{html, DOCTYPE};
use serde::{Deserialize, Serialize};

use crate::http::service;

use super::{asset, extract::RouterPath, sheet};

const OPENAPI_JSON_ROUTE: &str = "/openapi.json";

#[derive(Debug, Deserialize)]
pub struct Config {
	sheet: sheet::Config,
}

pub fn router(config: Config) -> Router<service::State> {
	let mut openapi = openapi::OpenApi::default();

	ApiRouter::new()
		.nest("/sheet", sheet::router(config.sheet))
		.nest("/asset", asset::router())
		.finish_api_with(&mut openapi, api_docs)
		.route(OPENAPI_JSON_ROUTE, get(openapi_json))
		.route("/docs", get(scalar))
		.layer(Extension(Arc::new(openapi)))
}

fn api_docs(mut api: TransformOpenApi) -> TransformOpenApi {
	let openapi = api.inner_mut();

	// Ensure we've not ended up with any trailing slashes.
	if let Some(paths) = openapi.paths.take() {
		openapi.paths = Some(openapi::Paths {
			paths: paths
				.paths
				.into_iter()
				.map(|(path, item)| (path.trim_end_matches('/').into(), item))
				.collect(),
			..paths
		})
	}

	api
}

// We want to avoid cloning the OpenApi struct, but need runtime information to
// know the location of the API routes within the routing heirachy. As such, we're
// overriding the `servers` here by including it after the flattened base.
#[derive(Serialize)]
struct OpenApiOverrides<'a> {
	#[serde(flatten)]
	base: &'a openapi::OpenApi,

	servers: &'a [openapi::Server],
}

#[debug_handler]
async fn openapi_json(
	RouterPath(router_path): RouterPath,
	Extension(openapi): Extension<Arc<openapi::OpenApi>>,
) -> impl IntoResponse {
	Json(OpenApiOverrides {
		base: &*openapi,
		servers: &[openapi::Server {
			url: router_path,
			..Default::default()
		}],
	})
	.into_response()
}

#[debug_handler]
async fn scalar(RouterPath(router_path): RouterPath) -> impl IntoResponse {
	html! {
		(DOCTYPE)
		html {
			head {
				title { "Boilmaster Documentation" }
				meta charset="utf-8";
				meta name="viewport" content="width=device-width, initial-scale=1";
			}
			body {
				script id="api-reference" data-url={ (router_path) (OPENAPI_JSON_ROUTE) } {}
				script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference" {}
			}
		}
	}
}

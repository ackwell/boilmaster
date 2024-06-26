use std::sync::Arc;

use aide::{
	axum::ApiRouter,
	openapi::{self, Tag},
	transform::TransformOpenApi,
};
use axum::{debug_handler, response::IntoResponse, routing::get, Extension, Json, Router};
use git_version::git_version;
use maud::{html, DOCTYPE};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::http::service;

use super::{asset, extract::RouterPath, sheet, version};

const OPENAPI_JSON_ROUTE: &str = "/openapi.json";

#[derive(Debug, Deserialize)]
pub struct Config {
	sheet: sheet::Config,
}

pub fn router(config: Config) -> Router<service::State> {
	let mut openapi = openapi::OpenApi::default();

	ApiRouter::new()
		.nest(
			"/asset",
			asset::router().with_path_items(|item| item.tag("assets")),
		)
		.nest(
			"/sheet",
			sheet::router(config.sheet).with_path_items(|item| item.tag("sheets")),
		)
		.nest(
			"/version",
			version::router().with_path_items(|item| item.tag("versions")),
		)
		.finish_api_with(&mut openapi, api_docs)
		.layer(CorsLayer::permissive())
		.route(OPENAPI_JSON_ROUTE, get(openapi_json))
		.route("/docs", get(scalar))
		.layer(Extension(Arc::new(openapi)))
}

fn api_docs(api: TransformOpenApi) -> TransformOpenApi {
	let mut api = api
		.title("boilmaster")
		.version(git_version!(prefix = "1-", fallback = "unknown"))
		.tag(Tag {
			name: "assets".into(),
			description: Some("Endpoints for accessing game data on a file-by-file basis. Commonly useful for fetching icons or other textures to display on the web.".into()),
			..Default::default()
		})
		.tag(Tag {
			name: "sheets".into(),
			description: Some("Endpoints for reading data from the game's static relational data store.".into()),
			..Default::default()
		})
		.tag(Tag {
			name: "versions".into(),
			description: Some("Endpoints for querying metadata about the versions recorded by the boilmaster system.".into()),
			..Default::default()
		});

	let openapi = api.inner_mut();

	let wildcard_regex = Regex::new(r#"\*(?<name>\w+)$"#).unwrap();

	if let Some(paths) = openapi.paths.take() {
		openapi.paths = Some(openapi::Paths {
			paths: paths
				.paths
				.into_iter()
				.map(|(path, item)| {
					// Ensure we've not ended up with any trailing slashes.
					let path = path.trim_end_matches('/');
					// Replace any missed `*wildcard`s with openapi compatible syntax
					let path = wildcard_regex.replace(path, "{$name}");
					(path.into(), item)
				})
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

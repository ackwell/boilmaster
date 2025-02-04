use std::sync::Arc;

use aide::{axum::ApiRouter, openapi, transform::TransformOpenApi};
use axum::{
	debug_handler,
	extract::{FromRef, State},
	http::Uri,
	response::IntoResponse,
	routing::get,
	Json, Router,
};
use git_version::git_version;
use maud::{html, DOCTYPE};
use regex::Regex;
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use crate::{http::HttpState, service::Service};

use super::{asset, read::RowReaderState, search, sheet, version};

const OPENAPI_JSON_ROUTE: &str = "/openapi.json";

#[derive(Debug, Deserialize)]
pub struct Config {
	asset: asset::Config,
	search: search::Config,
	sheet: sheet::Config,
}

#[derive(Clone, FromRef)]
pub struct ApiState {
	pub services: Service,
	pub reader_state: RowReaderState,
}

pub fn router(config: Config, state: HttpState) -> Router {
	let mut openapi = openapi::OpenApi::default();

	let state = ApiState {
		services: state.services,
		reader_state: RowReaderState::default(),
	};

	ApiRouter::new()
		.nest(
			"/asset",
			asset::router(config.asset, state.clone()).with_path_items(|item| item.tag("assets")),
		)
		.nest(
			"/search",
			search::router(config.search, state.clone()).with_path_items(|item| item.tag("search")),
		)
		.nest(
			"/sheet",
			sheet::router(config.sheet, state.clone()).with_path_items(|item| item.tag("sheets")),
		)
		.nest(
			"/version",
			version::router(state).with_path_items(|item| item.tag("versions")),
		)
		.finish_api_with(&mut openapi, api_docs)
		.route(
			OPENAPI_JSON_ROUTE,
			get(openapi_json).with_state(OpenApiState {
				openapi: Arc::new(openapi),
			}),
		)
		.layer(CorsLayer::permissive())
		.route("/docs", get(scalar))
}

fn api_docs(api: TransformOpenApi) -> TransformOpenApi {
	let mut api = api
		.title("boilmaster")
		.version(git_version!(prefix = "1-", fallback = "unknown"))
		.tag(openapi::Tag {
			name: "assets".into(),
			description: Some("Endpoints for accessing game data on a file-by-file basis. Commonly useful for fetching icons or other textures to display on the web.".into()),
			..Default::default()
		})
		.tag(openapi::Tag {
			name: "search".into(),
			description: Some("Endpoints for seaching and filtering the game's static relational data store.".into()),
			..Default::default()
		})
		.tag(openapi::Tag {
			name: "sheets".into(),
			description: Some("Endpoints for reading data from the game's static relational data store.".into()),
			..Default::default()
		})
		.tag(openapi::Tag {
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

#[derive(Clone, FromRef)]
struct OpenApiState {
	openapi: Arc<openapi::OpenApi>,
}

#[debug_handler]
async fn openapi_json(State(openapi): State<Arc<openapi::OpenApi>>) -> impl IntoResponse {
	Json(openapi.as_ref()).into_response()
}

#[debug_handler]
async fn scalar(uri: Uri) -> impl IntoResponse {
	html! {
		(DOCTYPE)
		html {
			head {
				title { "Boilmaster Documentation" }
				meta charset="utf-8";
				meta name="viewport" content="width=device-width, initial-scale=1";
			}
			body {
				script id="api-reference" data-url={ "." (OPENAPI_JSON_ROUTE) } {}
				// This script sets the configuration with a new server url based on
				// what the browser can see - this ensures that regardless of what
				// reverse proxies may be doing, the urls will be relative to the API's
				// mount point. I take no responsibility for any further fuckery.
				script {
					"var route = '" (uri) "';"
					r#"
					var serverUrl = location.origin + location.pathname.replace(new RegExp(route + '$'), '');
					var configuration = { servers: [{ url: serverUrl }] };
					document.getElementById('api-reference').dataset.configuration = JSON.stringify(configuration);
					"#
				}
				script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference" {}
			}
		}
	}
}

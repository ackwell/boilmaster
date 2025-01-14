use std::sync::Arc;

use aide::{axum::ApiRouter, openapi, transform::TransformOpenApi};
use axum::{
	debug_handler,
	extract::{FromRef, NestedPath, State},
	response::IntoResponse,
	routing::get,
	Json, Router,
};
use git_version::git_version;
use maud::{html, DOCTYPE};
use regex::Regex;
use serde::{Deserialize, Serialize};
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
		.layer(CorsLayer::permissive())
		.route(
			OPENAPI_JSON_ROUTE,
			get(openapi_json).with_state(OpenApiState {
				openapi: Arc::new(openapi),
			}),
		)
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
	nested_path: NestedPath,
	State(openapi): State<Arc<openapi::OpenApi>>,
) -> impl IntoResponse {
	Json(OpenApiOverrides {
		base: &*openapi,
		servers: &[openapi::Server {
			url: nested_path.as_str().to_string(),
			..Default::default()
		}],
	})
	.into_response()
}

#[debug_handler]
async fn scalar(nested_path: NestedPath) -> impl IntoResponse {
	html! {
		(DOCTYPE)
		html {
			head {
				title { "Boilmaster Documentation" }
				meta charset="utf-8";
				meta name="viewport" content="width=device-width, initial-scale=1";
			}
			body {
				script id="api-reference" data-url={ (nested_path.as_str()) (OPENAPI_JSON_ROUTE) } {}
				script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference" {}
			}
		}
	}
}

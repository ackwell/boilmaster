use std::{
	ffi::OsStr,
	hash::{Hash, Hasher},
	time::Duration,
};

use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	openapi,
	transform::TransformOperation,
	NoApi,
};
use axum::{
	debug_handler,
	extract::{FromRef, State},
	http::header,
	response::IntoResponse,
};
use axum_extra::{
	headers::{CacheControl, ContentType, ETag, IfNoneMatch},
	TypedHeader,
};
use reqwest::StatusCode;
use schemars::JsonSchema;
use seahash::SeaHasher;
use serde::Deserialize;
use strum::IntoEnumIterator;

use crate::{asset::Format, http::service::Service, version::VersionKey};

use super::{
	api::ApiState,
	error::Result,
	extract::{Path, Query, VersionQuery},
};

// NOTE: Bump this if changing any behavior that impacts output binary data for assets, to ensure ETag is cache-broken.
const ASSET_ETAG_VERSION: usize = 2;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	maxage: u64,
}

#[derive(Clone, FromRef)]
struct AssetState {
	services: Service,
	config: Config,
}

pub fn router(config: Config, state: ApiState) -> ApiRouter {
	let state = AssetState {
		services: state.services,
		config,
	};

	ApiRouter::new()
		.api_route("/", get_with(asset2, asset2_docs))
		.api_route("/map/:territory/:index", get_with(map, map_docs))
		// Fall back to the old asset endpoint for compatibility.
		.route("/*path", axum::routing::get(asset1))
		.with_state(state)
}

// Original asset endpoint based on a game path in the url path.

#[derive(Deserialize)]
struct Asset1Path {
	path: String,
}

#[derive(Deserialize)]
struct Asset1Query {
	format: Format,
}

#[debug_handler(state = AssetState)]
async fn asset1(
	Path(Asset1Path { path }): Path<Asset1Path>,
	query_version: VersionQuery,
	Query(Asset1Query { format }): Query<Asset1Query>,
	header_if_none_match: NoApi<Option<TypedHeader<IfNoneMatch>>>,
	state_service: State<Service>,
	state_config: State<Config>,
) -> Result<impl IntoApiResponse> {
	// The endpoints are nearly identical - just call through to the new endpoint with an emulated query.
	asset2(
		query_version,
		Query(AssetQuery { path, format }),
		header_if_none_match,
		state_service,
		state_config,
	)
	.await
}

/// Query parameters accepted by the asset endpoint.
#[derive(Deserialize, JsonSchema)]
struct AssetQuery {
	/// Game path of the asset to retrieve.
	#[schemars(example = "example_path")]
	path: String,

	/// Format that the asset should be converted into.
	#[schemars(example = "example_format")]
	format: Format,
}

fn example_path() -> &'static str {
	"ui/icon/051000/051474_hr1.tex"
}

fn example_format() -> Format {
	Format::Png
}

fn asset2_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("read an asset")
		.description("Read an asset from the game at the specified path, converting it into a usable format. If no valid conversion between the game file type and specified format exists, an error will be returned.")
		.response_with::<200, Vec<u8>, _>(|mut response| {
			response.inner().content = Format::iter()
				.map(|format| {
					(
						format_mime(format).to_string(),
						openapi::MediaType::default(),
					)
				})
				.collect();
			response
		})
		.response_with::<304, (), _>(|res| res.description("not modified"))
}

#[debug_handler(state = AssetState)]
async fn asset2(
	VersionQuery(version_key): VersionQuery,
	Query(AssetQuery { path, format }): Query<AssetQuery>,
	NoApi(header_if_none_match): NoApi<Option<TypedHeader<IfNoneMatch>>>,
	State(Service { asset, .. }): State<Service>,
	State(config): State<Config>,
) -> Result<impl IntoApiResponse> {
	let etag = etag(&path, format, version_key);

	// If the request came through with a passing ETag, we can skip doing any processing.
	if let Some(TypedHeader(if_none_match)) = header_if_none_match {
		if !if_none_match.precondition_passes(&etag) {
			return Ok(StatusCode::NOT_MODIFIED.into_response());
		}
	}

	// Perform the conversion.
	// TODO: can this be made async?
	let bytes = asset.convert(version_key, &path, format)?;

	// Try to derive a filename to use for the Content-Disposition header.
	let filepath = std::path::Path::new(&path).with_extension(format.extension());
	let disposition = match filepath.file_name().and_then(OsStr::to_str) {
		Some(name) => format!("inline; filename=\"{name}\""),
		None => "inline".to_string(),
	};

	// Set up the Cache-Control header based on configured max-age.
	let cache_control = CacheControl::new()
		.with_public()
		.with_immutable()
		.with_max_age(Duration::from_secs(config.maxage));

	let response = (
		TypedHeader(ContentType::from(format_mime(format))),
		// TypedHeader only has a really naive inline value with no ability to customise :/
		[(header::CONTENT_DISPOSITION, disposition)],
		TypedHeader(etag),
		TypedHeader(cache_control),
		bytes,
	);

	Ok(response.into_response())
}

fn format_mime(format: Format) -> mime::Mime {
	match format {
		Format::Png => mime::IMAGE_PNG,
	}
}

fn etag(path: &str, format: Format, version: VersionKey) -> ETag {
	let mut hasher = SeaHasher::new();
	path.hash(&mut hasher);
	format.extension().hash(&mut hasher);
	let resource_hash = hasher.finish();

	format!("\"{resource_hash:016x}.{version}.{ASSET_ETAG_VERSION}\"")
		.parse()
		.expect("malformed etag")
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MapPath {
	territory: String,
	index: String,
}

fn map_docs(operation: TransformOperation) -> TransformOperation {
	operation
}

#[debug_handler]
async fn map(
	Path(MapPath { territory, index }): Path<MapPath>,
	VersionQuery(version_key): VersionQuery,
	State(Service { asset, .. }): State<Service>,
) -> Result<impl IntoApiResponse> {
	// todo: caches and all that
	let bytes = asset.map(version_key, &territory, &index)?;

	let response = (
		TypedHeader(ContentType::png()),
		[(header::CONTENT_DISPOSITION, "inline")],
		bytes,
	);

	Ok(response.into_response())
}

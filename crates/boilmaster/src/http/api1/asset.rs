use std::{
	ffi::OsStr,
	hash::{Hash, Hasher},
	time::Duration,
};

use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	openapi,
	transform::TransformOperation,
};
use axum::{
	debug_handler,
	extract::{FromRef, OriginalUri, Request, State},
	http::{header, StatusCode},
	middleware,
	response::{IntoResponse, Response},
};
use axum_extra::{
	headers::{CacheControl, ContentType, ETag, HeaderMapExt, IfNoneMatch},
	TypedHeader,
};
use bm_asset::Format;
use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject},
	JsonSchema,
};
use seahash::SeaHasher;
use serde::{Deserialize, Serialize};

use crate::http::service::Service;

use super::{
	api::ApiState,
	error::Result,
	extract::{Path, Query, VersionQuery},
	jsonschema::impl_jsonschema,
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
		.layer(middleware::from_fn_with_state(state.clone(), cache_layer))
		.with_state(state)
}

// Original asset endpoint based on a game path in the url path.

#[derive(Deserialize)]
struct Asset1Path {
	path: String,
}

#[derive(Deserialize)]
struct Asset1Query {
	format: SchemaFormat,
}

#[debug_handler(state = AssetState)]
async fn asset1(
	Path(Asset1Path { path }): Path<Asset1Path>,
	query_version: VersionQuery,
	Query(Asset1Query { format }): Query<Asset1Query>,
	state_service: State<Service>,
) -> Result<impl IntoApiResponse> {
	// The endpoints are nearly identical - just call through to the new endpoint with an emulated query.
	asset2(
		query_version,
		Query(AssetQuery { path, format }),
		state_service,
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
	format: SchemaFormat,
}

fn example_path() -> &'static str {
	"ui/icon/051000/051474_hr1.tex"
}

#[derive(Serialize, Deserialize)]
#[repr(transparent)]
struct SchemaFormat(Format);

impl_jsonschema!(SchemaFormat, format_schema);
fn format_schema(_generator: &mut SchemaGenerator) -> Schema {
	Schema::Object(SchemaObject {
		instance_type: Some(InstanceType::String.into()),
		enum_values: Some(
			Format::iter()
				.map(|format| serde_json::to_value(format).expect("should not fail"))
				.collect(),
		),
		..Default::default()
	})
}

fn example_format() -> SchemaFormat {
	SchemaFormat(Format::Png)
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
	Query(AssetQuery {
		path,
		format: SchemaFormat(format),
	}): Query<AssetQuery>,
	State(Service { asset, .. }): State<Service>,
) -> Result<impl IntoApiResponse> {
	// Perform the conversion.
	// TODO: can this be made async?
	let bytes = asset.convert(version_key, &path, format)?;

	// Try to derive a filename to use for the Content-Disposition header.
	let filepath = std::path::Path::new(&path).with_extension(format.extension());
	let disposition = match filepath.file_name().and_then(OsStr::to_str) {
		Some(name) => format!("inline; filename=\"{name}\""),
		None => "inline".to_string(),
	};

	let response = (
		TypedHeader(ContentType::from(format_mime(format))),
		// TypedHeader only has a really naive inline value with no ability to customise :/
		[(header::CONTENT_DISPOSITION, disposition)],
		bytes,
	);

	Ok(response.into_response())
}

fn format_mime(format: Format) -> mime::Mime {
	match format {
		Format::Jpeg => mime::IMAGE_JPEG,
		Format::Png => mime::IMAGE_PNG,
		Format::Webp => "image/webp".parse().expect("mime parse should not fail"),
	}
}

/// Path segments expected by the asset map endpoint.
#[derive(Debug, Deserialize, JsonSchema)]
struct MapPath {
	/// Territory of the map to be retrieved. This typically takes the form of 4
	/// characters, [letter][number][letter][number]. See `Map`'s `Id` field for
	/// examples of possible combinations of `territory` and `index`.
	#[schemars(example = "example_territory")]
	territory: String,

	/// Index of the map within the territory. This invariably takes the form of a
	/// two-digit zero-padded number. See `Map`'s `Id` field for examples of
	/// possible combinations of `territory` and `index`.
	#[schemars(example = "example_index")]
	index: String,
}

fn example_territory() -> &'static str {
	"s1d1"
}

fn example_index() -> &'static str {
	"00"
}

fn map_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("compose a map")
		.description(
			"Retrieve the specified map, composing it from split source files if necessary.",
		)
		.response_with::<200, Vec<u8>, _>(|mut response| {
			let content = &mut response.inner().content;
			content.clear();
			content.insert(mime::IMAGE_JPEG.to_string(), openapi::MediaType::default());
			response
		})
		.response_with::<304, (), _>(|res| res.description("not modified"))
}

#[debug_handler]
async fn map(
	Path(MapPath { territory, index }): Path<MapPath>,
	VersionQuery(version_key): VersionQuery,
	State(Service { asset, .. }): State<Service>,
) -> Result<impl IntoApiResponse> {
	let bytes = asset.map(version_key, &territory, &index)?;

	let response = (
		TypedHeader(ContentType::jpeg()),
		[(
			header::CONTENT_DISPOSITION,
			format!("inline; filename=\"{territory}_{index}.jpg\""),
		)],
		bytes,
	);

	Ok(response.into_response())
}

async fn cache_layer(
	uri: OriginalUri,
	VersionQuery(version): VersionQuery,
	header_if_none_match: Option<TypedHeader<IfNoneMatch>>,
	State(config): State<Config>,
	request: Request,
	next: middleware::Next,
) -> Response {
	// Build ETag for this request.
	let mut hasher = SeaHasher::new();
	uri.hash(&mut hasher);
	let uri_hash = hasher.finish();

	let etag = format!("\"{uri_hash:016x}.{version}.{ASSET_ETAG_VERSION}\"")
		.parse::<ETag>()
		.expect("malformed etag");

	// If the request came through with a passing ETag, we can skip doing any processing.
	if let Some(TypedHeader(if_none_match)) = header_if_none_match {
		if !if_none_match.precondition_passes(&etag) {
			return StatusCode::NOT_MODIFIED.into_response();
		}
	}

	// ETag didn't match, pass down to the rest of the handlers.
	let mut response = next.run(request).await;

	// Add cache headers.
	let cache_control = CacheControl::new()
		.with_public()
		.with_immutable()
		.with_max_age(Duration::from_secs(config.maxage));

	let headers = response.headers_mut();
	headers.typed_insert(etag);
	headers.typed_insert(cache_control);

	response
}

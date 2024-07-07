use std::{
	ffi::OsStr,
	hash::{Hash, Hasher},
};

use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	openapi,
	transform::TransformOperation,
	NoApi,
};
use axum::{debug_handler, extract::State, http::header, response::IntoResponse};
use axum_extra::{
	headers::{ContentType, ETag, IfNoneMatch},
	TypedHeader,
};
use reqwest::StatusCode;
use schemars::JsonSchema;
use seahash::SeaHasher;
use serde::Deserialize;
use strum::IntoEnumIterator;

use crate::{asset::Format, http::service, version::VersionKey};

use super::{
	error::Result,
	extract::{Path, Query, VersionQuery},
};

// NOTE: Bump this if changing any behavior that impacts output binary data for assets, to ensure ETag is cache-broken.
const ASSET_ETAG_VERSION: usize = 2;

pub fn router() -> ApiRouter<service::State> {
	ApiRouter::new().api_route("/*path", get_with(asset, asset_docs))
}

/// Path variables accepted by the asset endpoint.
#[derive(Deserialize, JsonSchema)]
struct AssetPath {
	/// Game path of the asset to retrieve.
	#[schemars(example = "example_path")]
	path: String,
}

fn example_path() -> &'static str {
	"ui/icon/051000/051474_hr1.tex"
}

/// Query parameters accepted by the asset endpoint.
#[derive(Deserialize, JsonSchema)]
struct AssetQuery {
	/// Format that the asset should be converted into.
	#[schemars(example = "example_format")]
	format: Format,
}

fn example_format() -> Format {
	Format::Png
}

fn asset_docs(operation: TransformOperation) -> TransformOperation {
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

#[debug_handler(state = service::State)]
async fn asset(
	Path(AssetPath { path }): Path<AssetPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<AssetQuery>,
	NoApi(header_if_none_match): NoApi<Option<TypedHeader<IfNoneMatch>>>,
	State(asset): State<service::Asset>,
) -> Result<impl IntoApiResponse> {
	let format = query.format;

	let etag = etag(&path, format, version_key);

	if let Some(TypedHeader(if_none_match)) = header_if_none_match {
		if !if_none_match.precondition_passes(&etag) {
			return Ok(StatusCode::NOT_MODIFIED.into_response());
		}
	}

	let bytes = asset.convert(version_key, &path, format)?;

	let filepath = std::path::Path::new(&path).with_extension(format.extension());
	let disposition = match filepath.file_name().and_then(OsStr::to_str) {
		Some(name) => format!("inline; filename=\"{name}\""),
		None => "inline".to_string(),
	};

	Ok((
		TypedHeader(ContentType::from(format_mime(format))),
		// TypedHeader only has a really naive inline value with no ability to customise :/
		[(header::CONTENT_DISPOSITION, disposition)],
		TypedHeader(etag),
		bytes,
	)
		.into_response())
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

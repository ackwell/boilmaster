use std::{
	ffi::OsStr,
	hash::{Hash, Hasher},
};

use axum::{
	debug_handler,
	extract::State,
	http::header,
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use axum_extra::{
	headers::{ContentType, ETag, IfNoneMatch},
	TypedHeader,
};
use reqwest::StatusCode;
use seahash::SeaHasher;
use serde::Deserialize;

use crate::{asset::Format, http::service, version::VersionKey};

use super::{
	error::Result,
	extract::{Path, Query, VersionQuery},
};

pub fn router() -> Router<service::State> {
	Router::new().route("/*path", get(asset))
}

#[derive(Deserialize)]
struct AssetQuery {
	format: Format,
}

#[debug_handler(state = service::State)]
async fn asset(
	Path(path): Path<String>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<AssetQuery>,
	header_if_none_match: Option<TypedHeader<IfNoneMatch>>,
	State(asset): State<service::Asset>,
) -> Result<Response> {
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

	format!("\"{resource_hash:016x}.{version}\"")
		.parse()
		.expect("malformed etag")
}

use std::ffi::OsStr;

use axum::{
	debug_handler, extract::State, headers::ContentType, http::header, response::IntoResponse,
	routing::get, Router, TypedHeader,
};
use serde::Deserialize;

use crate::{asset::Format, http::service};

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
	State(asset): State<service::Asset>,
) -> Result<impl IntoResponse> {
	let format = query.format;

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
		bytes,
	))
}

fn format_mime(format: Format) -> mime::Mime {
	match format {
		Format::Png => mime::IMAGE_PNG,
	}
}

use axum::{debug_handler, extract::State, response::IntoResponse, routing::get, Json, Router};

use crate::utility::anyhow::Anyhow;

use super::{error::Result, extract::VersionQuery, service};

pub fn router() -> Router<service::State> {
	Router::new().route("/", get(list))
}

#[debug_handler(state = service::State)]
async fn list(
	VersionQuery(version_key): VersionQuery,
	State(data): State<service::Data>,
) -> Result<impl IntoResponse> {
	let excel = data.version(version_key)?.excel();

	let list = excel.list().anyhow()?;
	let mut names = list
		.iter()
		.map(|name| name.into_owned())
		.collect::<Vec<_>>();
	names.sort();

	Ok(Json(names))
}

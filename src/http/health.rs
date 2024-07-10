use axum::{debug_handler, extract::State, response::IntoResponse, routing::get, Router};
use reqwest::StatusCode;

use super::service;

pub fn router() -> Router<service::State> {
	Router::new()
		.route("/live", get(live))
		.route("/ready", get(ready))
}

#[debug_handler]
async fn live() -> impl IntoResponse {
	(StatusCode::OK, "LIVE")
}

#[debug_handler(state = service::State)]
async fn ready(
	State(asset): State<service::Asset>,
	State(data): State<service::Data>,
	State(schema): State<service::Schema>,
	State(search): State<service::Search>,
	State(version): State<service::Version>,
) -> impl IntoResponse {
	let ready =
		asset.ready() && data.ready() && schema.ready() && search.ready() && version.ready();
	match ready {
		true => (StatusCode::OK, "READY"),
		false => (StatusCode::SERVICE_UNAVAILABLE, "PENDING"),
	}
}

use axum::{
	debug_handler, extract::State, http::StatusCode, response::IntoResponse, routing::get, Router,
};

use super::{http::HttpState, service::Service};

pub fn router(state: HttpState) -> Router {
	Router::new()
		.route("/live", get(live))
		.route("/ready", get(ready).with_state(state))
}

#[debug_handler]
async fn live() -> impl IntoResponse {
	(StatusCode::OK, "LIVE")
}

#[debug_handler(state = HttpState)]
async fn ready(
	State(Service {
		asset,
		data,
		schema,
		search,
		version,
		..
	}): State<Service>,
) -> impl IntoResponse {
	let ready =
		asset.ready() && data.ready() && schema.ready() && search.ready() && version.ready();
	match ready {
		true => (StatusCode::OK, "READY"),
		false => (StatusCode::SERVICE_UNAVAILABLE, "PENDING"),
	}
}

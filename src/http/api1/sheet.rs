use axum::{debug_handler, response::IntoResponse, routing::get, Router};

pub fn router() -> Router {
	Router::new().route("/", get(list))
}

#[debug_handler]
async fn list() -> impl IntoResponse {
	"sheet list"
}

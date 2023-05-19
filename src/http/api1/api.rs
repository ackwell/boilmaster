use axum::Router;

use super::sheet;

pub fn router() -> Router {
	Router::new().nest("/sheet", sheet::router())
}

use axum::Router;

use super::{service, sheet};

pub fn router() -> Router<service::State> {
	Router::new().nest("/sheet", sheet::router())
}

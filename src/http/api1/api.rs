use axum::Router;
use serde::Deserialize;

use super::{asset, service, sheet};

#[derive(Debug, Deserialize)]
pub struct Config {
	sheet: sheet::Config,
}

pub fn router(config: Config) -> Router<service::State> {
	Router::new()
		.nest("/sheet", sheet::router(config.sheet))
		.nest("/asset", asset::router())
}

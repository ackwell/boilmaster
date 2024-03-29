use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use axum::{Router, Server};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use super::{
	admin,
	api1,
	// search,
	service,
};

#[derive(Debug, Deserialize)]
pub struct Config {
	admin: admin::Config,
	api1: api1::Config,

	address: Option<IpAddr>,
	port: u16,
}

pub async fn serve(
	cancel: CancellationToken,
	config: Config,
	data: service::Data,
	asset: service::Asset,
	schema: service::Schema,
	// search: service::Search,
	version: service::Version,
) -> Result<()> {
	let bind_address = SocketAddr::new(
		config.address.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
		config.port,
	);

	tracing::info!("http binding to {bind_address:?}");

	let router = Router::new()
		.nest("/admin", admin::router(config.admin))
		.nest("/api/1", api1::router(config.api1))
		// old:
		// .nest("/search", search::router())
		.layer(TraceLayer::new_for_http())
		.with_state(service::State {
			asset,
			data,
			schema,
			// search,
			version,
		});

	Server::bind(&bind_address)
		.serve(router.into_make_service())
		.with_graceful_shutdown(cancel.cancelled())
		.await
		.unwrap();

	Ok(())
}

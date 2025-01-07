use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Result;
use axum::{extract::FromRef, Router};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

use super::{admin, api1, health, service};

#[derive(Debug, Deserialize)]
pub struct Config {
	admin: admin::Config,
	api1: api1::Config,

	address: Option<IpAddr>,
	port: u16,
}

#[derive(Clone, FromRef)]
pub struct HttpState {
	pub services: service::Service,
}

pub async fn serve(
	cancel: CancellationToken,
	config: Config,
	asset: service::Asset,
	data: service::Data,
	read: service::Read,
	schema: service::Schema,
	search: service::Search,
	version: service::Version,
) -> Result<()> {
	let bind_address = SocketAddr::new(
		config.address.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
		config.port,
	);

	tracing::info!("http binding to {bind_address:?}");

	let state = HttpState {
		services: service::Service {
			asset,
			data,
			read,
			schema,
			search,
			version,
		},
	};

	let router = Router::new()
		.nest("/admin", admin::router(config.admin, state.clone()))
		.nest("/api/1", api1::router(config.api1, state.clone()))
		.nest("/health", health::router(state))
		.layer(
			// Set up tracing, but downgrade access logs to TRACE. Spans will remain
			// at DEBUG, allowing them to show up as relevant metadata for other
			// traces / failures in the system.
			TraceLayer::new_for_http()
				.on_request(DefaultOnRequest::new().level(Level::TRACE))
				.on_response(DefaultOnResponse::new().level(Level::TRACE))
				.on_failure(DefaultOnFailure::new().level(Level::TRACE)),
		);

	let listener = TcpListener::bind(bind_address).await.unwrap();
	axum::serve(listener, router)
		.with_graceful_shutdown(cancel.cancelled_owned())
		.await
		.unwrap();

	Ok(())
}

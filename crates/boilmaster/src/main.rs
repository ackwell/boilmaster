use std::sync::Arc;

use anyhow::Context;
use boilmaster::{http, tracing};
use figment::{
	providers::{Env, Format, Toml},
	Figment,
};
use futures::FutureExt;
use serde::Deserialize;
use tokio::signal;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
struct Config {
	// tracing: tracing::Config, - read individually.
	http: http::Config,
	read: bm_read::Config,
	version: bm_version::Config,
	schema: bm_schema::Config,
	search: bm_search::Config,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// Prepare the configuration hierarchy.
	// TODO: is it worth having a cli flag to specify the config path or is that just immense overkill?
	let figment = Figment::new()
		.merge(Toml::file("boilmaster.toml"))
		.merge(Env::prefixed("BM_").split("_"));

	// Initialise tracing before getting too far into bootstrapping the rest of
	// the application. We extract only the tracing configuration first, so that
	// the tracing library is bootstrapped before the rest of the configuration
	// is read, in case any of it traces.
	let tracing_config = figment
		.extract_inner::<tracing::Config>("tracing")
		.context("failed to initialize tracing config")?;
	tracing::init(tracing_config);

	// Load the rest of the configuration.
	let config = figment
		.extract::<Config>()
		.context("failed to extract config")?;

	let version = Arc::new(
		bm_version::Manager::new(config.version).context("failed to create version manager")?,
	);
	let data = Arc::new(bm_data::Data::new());
	let asset = Arc::new(bm_asset::Service::new(data.clone()));
	let read = Arc::new(bm_read::Read::new(config.read));
	let schema = Arc::new(
		bm_schema::Provider::new(config.schema, data.clone())
			.context("failed to create schema provider")?,
	);
	let search = Arc::new(
		bm_search::Search::new(config.search, data.clone(), schema.clone())
			.context("failed to create search service")?,
	);

	// Set up a cancellation token that will fire when a shutdown signal is recieved.
	let shutdown_token = shutdown_token();

	tokio::try_join!(
		version
			.start(shutdown_token.clone())
			.map(|result| result.context("version service")),
		data.start(shutdown_token.clone(), &version)
			.map(|result| result.context("data service")),
		schema
			.start(shutdown_token.clone())
			.map(|result| result.context("schema service")),
		search
			.start(shutdown_token.child_token())
			.map(|result| result.context("search service")),
		http::serve(
			shutdown_token,
			config.http,
			asset,
			data.clone(),
			read,
			schema.clone(),
			search.clone(),
			version.clone(),
		),
	)
	.context("failed to start server")?;

	Ok(())
}

fn shutdown_token() -> CancellationToken {
	// Create a token to represent the shutdown signal.
	let token = CancellationToken::new();

	// Set up a background task to wait for the signal with a copy of the token.
	let inner_token = token.clone();
	tokio::spawn(async move {
		shutdown_signal().await;
		inner_token.cancel();
	});

	// Return the pending token for use.
	token
}

async fn shutdown_signal() {
	let ctrl_c = async {
		signal::ctrl_c()
			.await
			.expect("Failed to install Ctrl+C handler.");
	};

	#[cfg(unix)]
	let terminate = async {
		signal::unix::signal(signal::unix::SignalKind::terminate())
			.expect("Failed to install SIGTERM handler.")
			.recv()
			.await
	};

	#[cfg(not(unix))]
	let terminate = std::future::pending::<()>();

	tokio::select! {
		_ = ctrl_c => {},
		_ = terminate => {},
	}

	::tracing::info!("shutdown signal received");
}

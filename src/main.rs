use std::sync::Arc;

use boilmaster::{
	asset,
	data,
	http,
	schema,
	// search,
	tracing,
	version,
};
use figment::{
	providers::{Env, Format, Toml},
	Figment,
};
use futures::TryFutureExt;
use serde::Deserialize;
use tokio::signal;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
struct Config {
	// tracing: tracing::Config, - read individually.
	http: http::Config,
	data: data::Config,
	version: version::Config,
	schema: schema::Config,
	// search: search::Config,
}

#[tokio::main]
async fn main() {
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
		.expect("Failed to initialize tracing config");
	tracing::init(tracing_config);

	// Load the rest of the configuration.
	let config = figment.extract::<Config>().expect("Failed to extract config");

	let version = Arc::new(version::Manager::new(config.version).expect("Failed to create version manager"));
	let data = Arc::new(data::Data::new(config.data));
	let asset = Arc::new(asset::Service::new(data.clone()));
	let schema =
		Arc::new(schema::Provider::new(config.schema, data.clone()).expect("Failed to create schema provider"));
	// let search = Arc::new(search::Search::new(config.search, data.clone()).expect("TODO"));

	// Set up a cancellation token that will fire when a shutdown signal is recieved.
	let shutdown_token = shutdown_token();

	tokio::try_join!(
		version.start(shutdown_token.clone()),
		data.start(shutdown_token.clone(), &version)
			.map_err(anyhow::Error::from),
		schema
			.start(shutdown_token.clone())
			.map_err(anyhow::Error::from),
		// search
		// 	.start(shutdown_token.child_token())
		// 	.map_err(anyhow::Error::from),
		http::serve(
			shutdown_token,
			config.http,
			data.clone(),
			asset,
			schema.clone(),
			// search.clone(),
			version.clone(),
		),
	)
	.expect("Failed to start server");
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

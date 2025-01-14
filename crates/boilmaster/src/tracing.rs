use std::{collections::HashMap, fmt, str::FromStr};

use serde::{de, Deserialize};
use tracing::{metadata::LevelFilter, Subscriber};
use tracing_subscriber::{
	filter, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt, Layer,
};

#[derive(Debug, Deserialize)]
pub struct Config {
	console: ConsoleConfig,
	stdout: StdoutConfig,
}

#[derive(Debug, Deserialize)]
struct ConsoleConfig {
	enabled: bool,
}

#[derive(Debug, Deserialize)]
struct StdoutConfig {
	enabled: bool,
	format: StdoutFormat,
	filters: TracingFilters,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum StdoutFormat {
	Full,
	Json,
}

#[derive(Debug, Deserialize)]
struct TracingFilters {
	default: ConfigLevelFilter,

	#[serde(flatten)]
	targets: HashMap<String, ConfigLevelFilter>,
}

#[repr(transparent)]
struct ConfigLevelFilter(LevelFilter);

impl From<ConfigLevelFilter> for LevelFilter {
	fn from(filter: ConfigLevelFilter) -> Self {
		filter.0
	}
}

impl fmt::Debug for ConfigLevelFilter {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl<'de> Deserialize<'de> for ConfigLevelFilter {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let string = String::deserialize(deserializer)?;
		let level_filter = LevelFilter::from_str(&string).map_err(de::Error::custom)?;
		Ok(Self(level_filter))
	}
}

pub fn init(config: Config) {
	// TODO: env filter (will need feature enabled). consider enabling pulling from log! too.
	// TODO: now that i have config working, is it worth using env filter here or should i handle it via config env?
	tracing_subscriber::registry()
		.with(tokio_console(config.console))
		.with(stdout(config.stdout))
		.init();
}

// TODO: Remove this when console subscriber is re-enabled
enum InfallibleLayer {}
impl<S> Layer<S> for InfallibleLayer
where
	S: Subscriber,
{
	fn on_enter(&self, _id: &tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {}
	fn on_exit(&self, _id: &tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {}
	fn on_close(&self, _id: tracing::span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {}
}

fn tokio_console<S>(config: ConsoleConfig) -> Option<impl Layer<S>>
where
	S: Subscriber + for<'a> LookupSpan<'a>,
{
	if !config.enabled {
		return None::<InfallibleLayer>;
	}

	// let layer = console_subscriber::spawn();

	// let filter = filter::Targets::new()
	// 	.with_target("tokio", LevelFilter::TRACE)
	// 	.with_target("runtime", LevelFilter::TRACE);

	// Some(layer.with_filter(filter))

	panic!("console subscriber is disabled pending axum 0.8 support");
}

fn stdout<S>(config: StdoutConfig) -> Option<impl Layer<S>>
where
	S: Subscriber + for<'a> LookupSpan<'a>,
{
	if !config.enabled {
		return None;
	}

	let layer = tracing_subscriber::fmt::layer();

	let layer = match config.format {
		StdoutFormat::Full => layer.boxed(),
		StdoutFormat::Json => layer.json().boxed(),
	};

	let filter = filter::Targets::new()
		.with_default(config.filters.default)
		.with_targets(config.filters.targets);

	Some(layer.with_filter(filter))
}

use std::{collections::HashMap, sync::Arc};

use futures::future::join_all;
use ironworks_schema::Schema;
use serde::Deserialize;
use tokio::{select, time};
use tokio_util::sync::CancellationToken;

use crate::{data, version::VersionKey};

use super::{
	error::{Error, Result},
	exdschema,
	specifier::CanonicalSpecifier,
	Specifier,
};

pub trait Source: Send + Sync {
	fn update(&self) -> Result<()>;

	fn canonicalize(&self, schema_version: Option<&str>, version_key: VersionKey)
		-> Result<String>;

	fn version(&self, version: &str) -> Result<Box<dyn Schema>>;
}

#[derive(Debug, Deserialize)]
pub struct Config {
	default: Specifier,
	interval: u64,

	exdschema: exdschema::Config,
}

// TODO: need a way to handle updating the repo
// TODO: look into moving sources into a channel so i'm not leaning on send+sync for other shit
pub struct Provider {
	default: Specifier,
	update_interval: u64,
	sources: HashMap<&'static str, Arc<dyn Source>>,
}

impl Provider {
	pub fn new(config: Config, data: Arc<data::Data>) -> Result<Self> {
		// TODO: at the moment this will hard fail if any source fails - should i make sources soft fail?
		Ok(Self {
			default: config.default,
			update_interval: config.interval,
			sources: HashMap::from([(
				"exdschema",
				boxed(exdschema::ExdSchema::new(config.exdschema, data)?),
			)]),
		})
	}

	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		select! {
			_ = self.start_inner() => Ok(()),
			_ = cancel.cancelled() => Ok(()),
		}
	}

	async fn start_inner(&self) {
		let mut interval = time::interval(time::Duration::from_secs(self.update_interval));
		interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

		loop {
			interval.tick().await;

			self.update().await;
		}
	}

	async fn update(&self) {
		tracing::info!("checking for schema updates");

		// TODO: Should this be spawn_blocking?
		let pending_updates = self.sources.iter().map(|(&name, source)| {
			let source = source.clone();
			tokio::spawn(async move { (name, source.update()) })
		});

		// Bubble panics, but log + ignore failures.
		for result in join_all(pending_updates).await {
			if let (name, Err(error)) = result.expect("schema update panic") {
				tracing::error!(%name, ?error, "schema update failed")
			}
		}
	}

	/// Canonicalise an optional specifier.
	pub fn canonicalize(
		&self,
		specifier: Option<Specifier>,
		version: VersionKey,
	) -> Result<CanonicalSpecifier> {
		let specifier = specifier.unwrap_or_else(|| self.default.clone());

		let source = self
			.sources
			.get(specifier.source.as_str())
			.ok_or_else(|| Error::UnknownSource(specifier.source.clone()))?;

		Ok(CanonicalSpecifier {
			source: specifier.source,
			version: source.canonicalize(specifier.version.as_deref(), version)?,
		})
	}

	pub fn schema(&self, specifier: CanonicalSpecifier) -> Result<Box<dyn Schema>> {
		let source = self
			.sources
			.get(specifier.source.as_str())
			.ok_or_else(|| Error::UnknownSource(specifier.source.clone()))?;
		source.version(&specifier.version)
	}
}

fn boxed(x: impl Source + 'static) -> Arc<dyn Source> {
	Arc::new(x)
}

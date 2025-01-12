use std::{
	borrow::Cow,
	collections::HashSet,
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
};

use anyhow::Context;
use bm_version::VersionKey;
use either::Either;
use ironworks::excel;
use itertools::Itertools;
use serde::Deserialize;
use tokio::select;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{data::Data, schema};

use super::{
	error::{Error, Result},
	internal_query::{pre, Normalizer},
	sqlite,
};

#[derive(Debug, Deserialize)]
pub struct Config {
	sqlite: sqlite::Config,
}

#[derive(Debug)]
pub enum SearchRequest {
	Query(SearchRequestQuery),
	Cursor(Uuid),
}

#[derive(Debug)]
pub struct SearchRequestQuery {
	pub version: VersionKey,
	pub query: pre::Node,
	pub language: excel::Language,
	pub sheets: Option<HashSet<String>>,
	pub schema: schema::CanonicalSpecifier,
}

#[derive(Debug)]
pub struct SearchResult {
	pub score: f32,
	// TODO: `String` here necessitates a copy of the sheet name for every result, which seems wasteful.
	pub sheet: String,
	pub row_id: u32,
	pub subrow_id: u16,
}

pub struct Search {
	ready: AtomicBool,

	provider: Arc<sqlite::Provider>,

	data: Arc<Data>,
	schema: Arc<schema::Provider>,
}

impl Search {
	pub fn new(config: Config, data: Arc<Data>, schema: Arc<schema::Provider>) -> Result<Self> {
		Ok(Self {
			ready: false.into(),
			provider: Arc::new(sqlite::Provider::new(config.sqlite, data.clone())?),
			data,
			schema,
		})
	}

	pub fn ready(&self) -> bool {
		self.ready.load(Ordering::Relaxed)
	}

	pub async fn start(&self, cancel: CancellationToken) -> Result<()> {
		let mut receiver = self.data.subscribe();
		self.ingest(cancel.child_token(), receiver.borrow().clone())
			.await?;

		loop {
			select! {
				Ok(_) = receiver.changed() => {
					self.ingest(cancel.child_token(), receiver.borrow().clone()).await?
				}
				_ = cancel.cancelled() => break,
			}
		}

		Ok(())
	}

	async fn ingest(&self, cancel: CancellationToken, versions: Vec<VersionKey>) -> Result<()> {
		// If there's no versions at all, there's nothing to do, bail.
		if versions.is_empty() {
			return Ok(());
		}

		fn collect_sheets(
			data: &Data,
			version: VersionKey,
		) -> Result<Vec<(VersionKey, excel::Sheet<String>)>> {
			let data_version = data
				.version(version)
				.context("announced for ingestion but not provided")?;
			let excel = data_version.excel();
			let list = excel.list().context("failed to obtain excel list")?;

			let sheets = list
				.iter()
				.map(|sheet_name| Ok((version, excel.sheet(sheet_name.to_string())?)))
				.collect::<Result<Vec<_>>>()
				.context("failed to obtain excel sheets")?;

			Ok(sheets)
		}

		// Get a list of all sheets in the provided versions.
		// TODO: This has more `.collect`s than i'd like, but given it's a fairly cold path, probably isn't a problem.
		let (sheets, errors): (Vec<_>, Vec<_>) = versions
			.into_iter()
			.map(|version| -> Result<_> {
				Ok(collect_sheets(&self.data, version).with_context(|| {
					format!("failed to prepare version {version} for ingestion")
				})?)
			})
			.flatten_ok()
			.partition_result();

		// If there's any version-wide errors, trace them out instead of lifting the
		// error - a hard failure here will nuke the entire server.
		for error in errors {
			tracing::error!("{error:?}");
		}

		// Fire off the ingestion in the provider.
		Arc::clone(&self.provider).ingest(cancel, sheets).await?;

		// At least one ingestion has occured, the service can be considered ready.
		self.ready.store(true, Ordering::Relaxed);

		tracing::info!("search ingestion complete");

		Ok(())
	}

	pub async fn search(
		&self,
		request: SearchRequest,
		limit: usize,
	) -> Result<(Vec<SearchResult>, Option<Uuid>)> {
		// Translate the request into the format used by providers.
		let provider_request = match request {
			SearchRequest::Query(query) => self.normalize_request_query(query)?,
			SearchRequest::Cursor(uuid) => sqlite::SearchRequest::Cursor(uuid),
		};

		// Execute the search.
		self.provider.search(provider_request, limit).await
	}

	fn normalize_request_query(&self, query: SearchRequestQuery) -> Result<sqlite::SearchRequest> {
		// Get references to the game data we'll need.
		let excel = self
			.data
			.version(query.version)
			.with_context(|| format!("data for version {} not ready", query.version))?
			.excel();
		let list = excel.list()?;

		// Build the helpers for this search call.
		let schema = self.schema.schema(query.schema)?;
		let normalizer = Normalizer::new(&excel, schema.as_ref());

		// Get an iterator over the provided sheet filter, falling back to the full list of sheets.
		let sheet_names = query
			.sheets
			.map(|filter| Either::Left(filter.into_iter().map(Cow::from)))
			.unwrap_or_else(|| Either::Right(list.iter()));

		let normalized_queries = sheet_names
			.map(|name| {
				let normalized_query = normalizer.normalize(&query.query, &name, query.language)?;
				Ok((name.to_string(), normalized_query))
			})
			// TODO: This is filtering out non-fatal errors. To raise as warnings, these will need to be split out at this point.
			.filter(|query| match query {
				Err(Error::Failure(_)) | Ok(_) => true,
				Err(_) => false,
			})
			.collect::<Result<Vec<_>>>()?;

		Ok(sqlite::SearchRequest::Query {
			version: query.version,
			queries: normalized_queries,
		})
	}
}

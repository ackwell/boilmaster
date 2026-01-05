use std::{collections::HashSet, str::FromStr};

use aide::{
	axum::{ApiRouter, IntoApiResponse, routing::get_with},
	transform::TransformOperation,
};
use axum::{
	Json, debug_handler,
	extract::{FromRef, State},
};
use bm_search::{SearchRequest as InnerSearchRequest, SearchRequestQuery};
use bm_version::VersionKey;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::Service;

use super::{
	api::ApiState,
	error::{Error, Result},
	extract::{Query, VersionQuery},
	query::QueryString,
	read::{RowReader, RowReaderConfig, RowReaderState, RowResult, Specifiers},
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	#[serde(flatten)]
	reader: RowReaderConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
	depth: u8,
}

#[derive(Clone, FromRef)]
struct RowsState {
	services: Service,
	reader_config: RowReaderConfig,
	reader_state: RowReaderState,
	limit_config: LimitConfig,
}

pub fn router(config: Config, state: ApiState) -> ApiRouter {
	let state = RowsState {
		services: state.services,
		reader_config: config.reader,
		reader_state: state.reader_state,
		limit_config: config.limit,
	};

	ApiRouter::new().api_route("/", get_with(search, search_docs).with_state(state))
}

/// Query paramters accepted by the search endpoint.
#[derive(Debug, Deserialize, JsonSchema)]
struct SearchQuery {
	/// Search query to execute in this request. Must be specified if not querying
	/// a cursor. URL special characters, such as `+`, must be escaped to prevent
	/// mis-parses of the query.
	query: Option<QueryString>,

	/// List of excel sheets that the query should be run against. At least one
	/// must be specified if not querying a cursor.
	sheets: Option<String>,

	/// Continuation token to retrieve further results from a prior search
	/// request. If specified, takes priority over `query`.
	cursor: Option<Uuid>,

	/// Maximum number of rows to return. To paginate, provide the cursor token
	/// provided in `next` to the `cursor` parameter.
	limit: Option<usize>,
}

/// Response structure for the search endpoint.
#[derive(Serialize, JsonSchema)]
struct SearchResponse {
	/// A cursor that can be used to retrieve further results if available.
	#[serde(skip_serializing_if = "Option::is_none")]
	next: Option<Uuid>,

	#[serde(flatten)]
	specifiers: Specifiers,

	/// Array of results found by the query, sorted by their relevance.
	results: Vec<SearchResult>,
}

/// Result found by a search query, hydrated with data from the underlying excel
/// row the result represents.
#[derive(Debug, Serialize, JsonSchema)]
struct SearchResult {
	/// Relevance score for this entry.
	///
	/// These values only loosely represent the relevance of an entry to the
	/// search query. No guarantee is given that the discrete values, nor
	/// resulting sort order, will remain stable.
	score: f32,

	/// Excel sheet this result was found in.
	sheet: String,

	#[serde(flatten)]
	row: RowResult,
}

fn search_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("execute a search query")
		.description(
			"Fetch information about rows and their related data that match the provided search query.",
		)
		.response_with::<200, Json<SearchResponse>, _>(|response| {
			response.example(SearchResponse {
				next: Some(Uuid::from_str("bbe61a5e-7d22-41ec-9f5a-711c967c5624").expect("static")),
				specifiers: Specifiers {
					schema: bm_schema::CanonicalSpecifier {
						source: "source".into(),
						version: "version".into(),
					},
					version: VersionKey::from_str("f815390159effefd").expect("static"),
				},
				results: vec![SearchResult {
					score: 1.413,
					sheet: "SheetName".into(),
					row: RowResult::example(1),
				}],
			})
		})
}

#[debug_handler(state = RowsState)]
async fn search(
	// TODO: this is a second versionquery extract for this, and it is being run twice. it's idempotent, but would be good to avoid
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<SearchQuery>,
	State(Service { search, .. }): State<Service>,
	State(config): State<LimitConfig>,
	mut reader: RowReader,
) -> Result<impl IntoApiResponse> {
	// Resolve search request into something the search service understands.
	// TODO: seperate fn?
	let request = match query.cursor {
		// Cursor always has priority
		Some(cursor) => InnerSearchRequest::Cursor(cursor),
		None => {
			let Some(search_query) = query.query else {
				return Err(Error::Invalid(
					"search queries must contain a query or cursor".into(),
				));
			};

			// TODO: This can be made optional, which will allow search queries that can theoretically search the entire db in one massive query. Will need extensive testing.
			let Some(sheets) = query.sheets else {
				return Err(Error::Invalid(
					"query-based searches must specify a list of sheets to search".into(),
				));
			};

			let sheets = sheets
				.split(',')
				.map(|sheet_name| sheet_name.to_owned())
				.collect::<HashSet<_>>();

			InnerSearchRequest::Query(SearchRequestQuery {
				version: version_key,
				query: search_query.into(),
				language: reader.language,
				sheets: Some(sheets),
				schema: reader.specifiers.schema.clone(),
			})
		}
	};

	let limit = query.limit.unwrap_or(config.default).min(config.max);

	// Run the actual search request.
	let (results, next_cursor) = search.search(request, limit).await?;

	let http_results = results
		.into_iter()
		.map(|result| {
			let row =
				reader.read_row(&result.sheet, result.row_id, result.subrow_id, config.depth)?;

			Ok(SearchResult {
				score: result.score,
				sheet: result.sheet,
				row,
			})
		})
		.collect::<Result<Vec<_>>>()?;

	Ok(Json(SearchResponse {
		next: next_cursor,
		specifiers: reader.specifiers,
		results: http_results,
	}))
}

use std::collections::{HashMap, HashSet};

use aide::axum::ApiRouter;
use axum::{debug_handler, extract::State, response::IntoResponse, routing::get, Extension, Json};
use ironworks::{excel::Language, file::exh};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
	http::service,
	read, schema,
	search::{SearchRequest as InnerSearchRequest, SearchRequestQuery},
	utility::anyhow::Anyhow,
};

use super::{
	error::Result,
	extract::{Query, VersionQuery},
	filter::FilterString,
	query::QueryString,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	filter: HashMap<String, FilterString>,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
	depth: u8,
}

pub fn router(config: Config) -> ApiRouter<service::State> {
	ApiRouter::new()
		.route("/", get(search))
		.layer(Extension(config))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
	language: Option<read::LanguageString>,

	schema: Option<schema::Specifier>,

	#[serde(flatten)]
	request: SearchRequest,

	fields: Option<FilterString>,

	limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SearchRequest {
	Query {
		query: QueryString,
		// TODO: This can be made optional, which will allow search queries that can theoretically search the entire db in one massive query. Will need extensive testing.
		sheets: String,
	},
	Cursor {
		cursor: Uuid,
	},
}

#[derive(Serialize)]
struct SearchResponse {
	next: Option<Uuid>,
	schema: schema::CanonicalSpecifier,
	results: Vec<SearchResult>,
}

// TODO: This is fairly duplicated with sheet::RowResult, which itself flags duplication with structures in read::. There's a degree of deduplication that potentially needs to happen, with note that over-indexing on that is problematic for api evolution.
#[derive(Debug, Serialize)]
struct SearchResult {
	score: f32,
	sheet: String,
	row_id: u32,
	#[serde(skip_serializing_if = "Option::is_none")]
	subrow_id: Option<u16>,
	fields: ValueString,
}

#[debug_handler(state = service::State)]
async fn search(
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<SearchQuery>,
	State(data): State<service::Data>,
	State(read): State<service::Read>,
	State(schema_provider): State<service::Schema>,
	State(search): State<service::Search>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoResponse> {
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(Language::from)
		.unwrap_or_else(|| read.default_language());

	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;
	let schema = schema_provider.schema(schema_specifier.clone())?;

	// Resolve search request into something the search service understands.
	// TODO: seperate fn?
	let request = match query.request {
		SearchRequest::Cursor { cursor } => InnerSearchRequest::Cursor(cursor),
		SearchRequest::Query {
			query: search_query,
			sheets,
		} => {
			let sheets = sheets
				.split(',')
				.map(|sheet_name| sheet_name.to_owned())
				.collect::<HashSet<_>>();

			InnerSearchRequest::Query(SearchRequestQuery {
				version: version_key,
				query: search_query.into(),
				language,
				sheets: Some(sheets),
				schema: schema_specifier.clone(),
			})
		}
	};

	let limit = query
		.limit
		.unwrap_or(config.limit.default)
		.min(config.limit.max);

	// Run the actual search request.
	let (results, next_cursor) = search.search(request, limit).await?;

	// Read and build result structures.
	let filter = query
		.fields
		.or_else(|| config.filter.get(&schema_specifier.source).cloned())
		.map(|filter_string| filter_string.to_filter(language))
		.unwrap_or(Ok(read::Filter::All))?;

	let http_results = results
		.into_iter()
		.map(|result| {
			let fields = read.read(
				&excel,
				schema.as_ref(),
				&result.sheet,
				result.row_id,
				result.subrow_id,
				language,
				&filter,
				config.limit.depth,
			)?;

			// TODO: this is pretty wasteful. Return from read::? ehh...
			let kind = excel.sheet(&result.sheet).anyhow()?.kind().anyhow()?;

			Ok(SearchResult {
				score: result.score,
				sheet: result.sheet,
				row_id: result.row_id,
				subrow_id: match kind {
					exh::SheetKind::Subrows => Some(result.subrow_id),
					_ => None,
				},
				fields: ValueString(fields, language),
			})
		})
		.collect::<Result<Vec<_>>>()?;

	Ok(Json(SearchResponse {
		next: next_cursor,
		schema: schema_specifier,
		results: http_results,
	}))
}

use std::collections::HashSet;

use aide::axum::ApiRouter;
use axum::{debug_handler, extract::State, response::IntoResponse, routing::get, Json};
use ironworks::excel::Language;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
	http::service,
	read::LanguageString,
	schema,
	search::{query, SearchRequest as InnerSearchRequest, SearchRequestQuery},
};

use super::{
	error::Result,
	extract::{Query, VersionQuery},
};

pub fn router() -> ApiRouter<service::State> {
	ApiRouter::new().route("/", get(search))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
	#[serde(flatten)]
	request: SearchRequest,

	limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SearchRequest {
	Query {
		query: query::Node,
		sheets: Option<String>,
	},
	Cursor {
		cursor: Uuid,
	},
}

// TODO: reuse this with sheets
#[derive(Deserialize)]
struct SchemaQuery {
	schema: Option<schema::Specifier>,
}

#[derive(Deserialize)]
struct LanguageQuery {
	language: Option<LanguageString>,
}

// TODO: flesh this out - at the moment it's just a 1:1 of searchresult, pending ideas on how to field filter for search results across multiple indices
#[derive(Debug, Serialize)]
struct SearchResult {
	score: f32,
	sheet: String,
	row_id: u32,
	subrow_id: u16,
}

#[debug_handler(state = service::State)]
async fn search(
	VersionQuery(version_key): VersionQuery,
	Query(search_query): Query<SearchQuery>,
	Query(schema_query): Query<SchemaQuery>,
	Query(language_query): Query<LanguageQuery>,
	State(read): State<service::Read>,
	State(schema_provider): State<service::Schema>,
	State(search): State<service::Search>,
) -> Result<impl IntoResponse> {
	let language = language_query
		.language
		.map(Language::from)
		.unwrap_or_else(|| read.default_language());

	// TODO: this should probably be in a seperate function
	let request = match search_query.request {
		SearchRequest::Cursor { cursor } => InnerSearchRequest::Cursor(cursor),
		SearchRequest::Query { query, sheets } => {
			let sheets = sheets.map(|encoded| {
				// TODO: I imagine comma-seperated stuff might be relatively common; make a deser helper (probs can trait it up so any fromiter<string> can deser using this pattern)
				encoded
					.split(',')
					.map(|x| x.to_owned())
					.collect::<HashSet<_>>()
			});

			let schema_specifier =
				schema_provider.canonicalize(schema_query.schema, version_key)?;
			let schema = schema_provider.schema(schema_specifier)?;

			InnerSearchRequest::Query(SearchRequestQuery {
				version: version_key,
				query,
				language,
				sheets,
				schema,
			})
		}
	};

	let (results, next_cursor) = search.search(request, search_query.limit).await?;

	let http_results = results
		.into_iter()
		.map(|result| SearchResult {
			score: result.score,
			sheet: result.sheet,
			row_id: result.row_id,
			subrow_id: result.subrow_id,
		})
		.collect::<Vec<_>>();

	Ok(Json((next_cursor, http_results)))
}

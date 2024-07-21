use std::{
	collections::{HashMap, HashSet},
	str::FromStr,
};

use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	transform::TransformOperation,
};
use anyhow::anyhow;
use axum::{debug_handler, extract::State, Extension, Json};
use ironworks::{excel, file::exh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
	http::service,
	read, schema,
	search::{SearchRequest as InnerSearchRequest, SearchRequestQuery},
	utility::anyhow::Anyhow,
};

use super::{
	error::{Error, Result},
	extract::{Query, VersionQuery},
	filter::FilterString,
	query::QueryString,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	fields: HashMap<String, FilterString>,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
	depth: u8,
}

pub fn router(config: Config) -> ApiRouter<service::State> {
	ApiRouter::new()
		.api_route("/", get_with(search, search_docs))
		.layer(Extension(config))
}

/// Query paramters accepted by the search endpoint.
#[derive(Debug, Deserialize, JsonSchema)]
struct SearchQuery {
	/// Language to use for search query and result fields when no language is
	/// otherwise specified.
	language: Option<read::LanguageString>,

	/// Schema that the search query and result fields should be read in
	/// accordance with.
	schema: Option<schema::Specifier>,

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

	/// Data fields to read for results found by the search.
	fields: Option<FilterString>,

	/// Maximum number of rows to return. To paginate, provide the cursor token
	/// provided in `next` to the `cursor` paramter.
	limit: Option<usize>,
}

/// Response structure for the search endpoint.
#[derive(Serialize, JsonSchema)]
struct SearchResponse {
	/// A cursor that can be used to retrieve further results if available.
	#[serde(skip_serializing_if = "Option::is_none")]
	next: Option<Uuid>,

	/// The canonical specifier for the schema used in this response.
	#[schemars(with = "String")]
	schema: schema::CanonicalSpecifier,

	/// Array of results found by the query, sorted by their relevance.
	results: Vec<SearchResult>,
}

// TODO: This is fairly duplicated with sheet::RowResult, which itself flags duplication with structures in read::. There's a degree of deduplication that potentially needs to happen, with note that over-indexing on that is problematic for api evolution.
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

	/// Row ID of this result.
	row_id: u32,

	/// Subrow ID of this result, when relevant.
	#[serde(skip_serializing_if = "Option::is_none")]
	subrow_id: Option<u16>,

	/// Field values for this row, according to the current schema and field filter.
	fields: ValueString,
}

fn search_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("execute a search query")
		.description("Fetch information about rows and their related data that match the provided search query.")
		.response_with::<200, Json<SearchResponse>, _>(|response| {
			response.example(SearchResponse {
				next: Some(Uuid::from_str("bbe61a5e-7d22-41ec-9f5a-711c967c5624").expect("static")),
				schema: schema::CanonicalSpecifier{
					source: "source".into(),
					version: "version".into()
				},
				results: vec![SearchResult {
					score: 1.413,
					sheet: "SheetName".into(),
					row_id: 1,
					subrow_id: None,
					fields: ValueString(
						read::Value::Struct(HashMap::from([(
							read::StructKey {
								name: "FieldName".into(),
								language: excel::Language::English,
							},
							read::Value::Scalar(excel::Field::U32(14)),
						)])),
						excel::Language::English,
					),
				}],
			})
		})
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
) -> Result<impl IntoApiResponse> {
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(excel::Language::from)
		.unwrap_or_else(|| read.default_language());

	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;
	let schema = schema_provider.schema(schema_specifier.clone())?;

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
		.or_else(|| config.fields.get(&schema_specifier.source).cloned())
		.map(|filter_string| filter_string.to_filter(language))
		.ok_or_else(|| {
			Error::Other(anyhow!(
				"missing default search fields for {}",
				schema_specifier.source
			))
		})??;

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

use std::{num::ParseIntError, str::FromStr};

use aide::{
	axum::{ApiRouter, IntoApiResponse, routing::get_with},
	transform::TransformOperation,
};
use axum::{
	Json, debug_handler,
	extract::{FromRef, State},
};
use either::Either;
use schemars::{
	JsonSchema,
	r#gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::service::Service;

use super::{
	api::ApiState,
	error::{Error, Result},
	extract::{Path, Query, VersionQuery},
	jsonschema::impl_jsonschema,
	read::{RowReader, RowReaderConfig, RowReaderState, RowResult},
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	list: RowReaderConfig,
	entry: RowReaderConfig,
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

pub fn router(config: Config, api_state: ApiState) -> ApiRouter {
	ApiRouter::new()
		.api_route("/", get_with(list, list_docs).with_state(api_state.clone()))
		.api_route(
			"/{sheet}",
			get_with(sheet, sheet_docs).with_state(RowsState {
				services: api_state.services.clone(),
				reader_config: config.list,
				reader_state: api_state.reader_state.clone(),
				limit_config: config.limit.clone(),
			}),
		)
		.api_route(
			"/{sheet}/{row}",
			get_with(row, row_docs).with_state(RowsState {
				services: api_state.services,
				reader_config: config.entry,
				reader_state: api_state.reader_state,
				limit_config: config.limit,
			}),
		)
}

/// Response structure for the list endpoint.
#[derive(Serialize, JsonSchema)]
struct ListResponse {
	/// Array of sheets known to the API.
	sheets: Vec<SheetMetadata>,
}

/// Metadata about a single sheet.
#[derive(Serialize, JsonSchema)]
struct SheetMetadata {
	/// The name of the sheet.
	name: String,
}

fn list_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list sheets")
		.description("List known excel sheets that can be read by the API.")
		.response_with::<200, Json<ListResponse>, _>(|response| {
			response.example(ListResponse {
				sheets: vec![
					SheetMetadata {
						name: "Action".into(),
					},
					SheetMetadata {
						name: "Item".into(),
					},
					SheetMetadata {
						name: "Status".into(),
					},
				],
			})
		})
}

#[debug_handler(state = ApiState)]
async fn list(
	VersionQuery(version_key): VersionQuery,
	State(Service { data, .. }): State<Service>,
) -> Result<Json<ListResponse>> {
	let excel = data.version(version_key)?.excel();

	let list = excel.list()?;
	let mut names = list
		.iter()
		.map(|name| name.into_owned())
		.collect::<Vec<_>>();
	names.sort();

	let metadata = names
		.into_iter()
		.map(|name| SheetMetadata { name })
		.collect();

	Ok(Json(ListResponse { sheets: metadata }))
}

/// Path variables accepted by the sheet endpoint.
#[derive(Deserialize, JsonSchema)]
struct SheetPath {
	/// Name of the sheet to read.
	sheet: String,
}

#[derive(Debug, PartialEq, PartialOrd)]
struct RowSpecifier {
	row_id: u32,
	subrow_id: u16,
}

impl FromStr for RowSpecifier {
	type Err = ParseIntError;

	fn from_str(string: &str) -> Result<Self, Self::Err> {
		let out = match string.split_once(':') {
			Some((row_id, subrow_id)) => Self {
				row_id: row_id.parse()?,
				subrow_id: subrow_id.parse()?,
			},
			None => Self {
				row_id: string.parse()?,
				subrow_id: 0,
			},
		};

		Ok(out)
	}
}

impl<'de> Deserialize<'de> for RowSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

impl_jsonschema!(RowSpecifier, rowspecifier_schema);
fn rowspecifier_schema(_generator: &mut SchemaGenerator) -> Schema {
	Schema::Object(SchemaObject {
		instance_type: Some(InstanceType::String.into()),
		string: Some(
			StringValidation {
				pattern: Some("^\\d+(:\\d+)?$".into()),
				..Default::default()
			}
			.into(),
		),
		..Default::default()
	})
}

/// Query parameters accepted by the sheet endpoint.
#[derive(Deserialize, JsonSchema)]
struct SheetQuery {
	/// Rows to fetch from the sheet, as a comma-separated list. Behavior is undefined if both `rows` and `after` are provided.
	#[serde(default, deserialize_with = "deserialize_rows")]
	#[schemars(schema_with = "rows_schema")]
	rows: Option<Vec<RowSpecifier>>,

	/// Maximum number of rows to return. To paginate, provide the last returned row to the next request's `after` parameter.
	limit: Option<usize>,

	/// Fetch rows after the specified row. Behavior is undefined if both `rows` and `after` are provided.
	after: Option<RowSpecifier>,
}

// TODO: this can probably be made as a general purpose "comma seperated" deserializer struct
fn deserialize_rows<'de, D>(deserializer: D) -> Result<Option<Vec<RowSpecifier>>, D::Error>
where
	D: Deserializer<'de>,
{
	let maybe_raw = Option::<String>::deserialize(deserializer)?;
	let raw = match maybe_raw.as_deref() {
		None | Some("") => return Ok(None),
		Some(value) => value,
	};

	// TODO: maybe use warnings for these too?
	let parsed = raw
		.split(',')
		.map(|x| x.parse())
		.collect::<Result<_, _>>()
		.map_err(de::Error::custom)?;

	Ok(Some(parsed))
}

fn rows_schema(_generator: &mut SchemaGenerator) -> Schema {
	Schema::Object(SchemaObject {
		instance_type: Some(InstanceType::String.into()),
		string: Some(
			StringValidation {
				pattern: Some("^\\d+(:\\d+)?(,\\d+(:\\d+)?)*$".into()),
				..Default::default()
			}
			.into(),
		),
		..Default::default()
	})
}

/// Response structure for the sheet endpoint.
#[derive(Serialize, JsonSchema)]
struct SheetResponse {
	/// The canonical specifier for the schema used in this response.
	#[schemars(with = "String")]
	schema: bm_schema::CanonicalSpecifier,

	/// Array of rows retrieved by the query.
	rows: Vec<RowResult>,
}

fn sheet_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list rows in a sheet")
		.description("Read information about one or more rows and their related data.")
		.response_with::<200, Json<SheetResponse>, _>(|response| {
			response.example(SheetResponse {
				schema: bm_schema::CanonicalSpecifier {
					source: "source".into(),
					version: "version".into(),
				},
				rows: vec![RowResult::example(1), RowResult::example(2)],
			})
		})
}

#[debug_handler(state = RowsState)]
async fn sheet(
	Path(path): Path<SheetPath>,
	Query(query): Query<SheetQuery>,
	State(config): State<LimitConfig>,
	reader: RowReader,
) -> Result<impl IntoApiResponse> {
	// Get a reference to the sheet we'll be reading from.
	// TODO: should this be in super::error as a default extract? minus the sheet specialised case, that is
	let sheet = reader
		.excel
		.sheet(&path.sheet)
		.map_err(|error| match error {
			ironworks::Error::NotFound(ironworks::ErrorValue::Sheet(..)) => {
				Error::NotFound(error.to_string())
			}
			other => Error::Other(other.into()),
		})?
		.with_default_language(reader.language);

	// Iterate over the sheet, building row results.
	let sheet_iterator = match query.rows {
		// One or more row specifiers were provided, iterate over those specifically.
		Some(specifiers) => Either::Left(specifiers.into_iter()),

		// None were provided, iterate over the sheet itself.
		// TODO: Currently, read:: does _all_ the row fetching itself, which means that we're effectively iterating the sheet here _just_ to get the row IDs, then re-fetching in the read:: code. This... probably isn't too problematic, but worth considering how to approach more betterer. If read:: can be modified to take a row, then the Some() case above can be specailised to the read-row logic and this case can be simplified.
		None => Either::Right(sheet.into_iter().filter_map(|row| match row {
			Err(error) => {
				// TODO: This is pretty horrid, but good enough for the sake of surfacing without more changes.
				tracing::error!("got error while iterating sheet: {error}");
				None
			}
			Ok(row) => Some(RowSpecifier {
				row_id: row.row_id(),
				subrow_id: row.subrow_id(),
			}),
		})),
	};

	// Paginate the results.
	let limit = query.limit.unwrap_or(config.default).min(config.max);
	let sheet_iterator = sheet_iterator
		// TODO: Improve this - introducing an explicit "after" method on a sheet iterator would allow skipping a lot of busywork. As-is, this is fetching every single row's data.
		.skip_while(|specifier| Some(specifier) <= query.after.as_ref())
		.take(limit);

	// Build Results for the targeted rows.
	let sheet_iterator = sheet_iterator.map(|specifier| {
		reader.read_row(
			&path.sheet,
			specifier.row_id,
			specifier.subrow_id,
			config.depth,
		)
	});

	let rows = sheet_iterator.collect::<Result<Vec<_>>>()?;

	let response = SheetResponse {
		schema: reader.schema_specifier,
		rows,
	};

	Ok(Json(response))
}

/// Path variables accepted by the row endpoint.
#[derive(Deserialize, JsonSchema)]
struct RowPath {
	/// Name of the sheet to read.
	sheet: String,
	/// Row to read.
	row: RowSpecifier,
}

/// Response structure for the row endpoint.
#[derive(Serialize, JsonSchema)]
struct RowResponse {
	/// The canonical specifier for the schema used in this response.
	#[schemars(with = "String")]
	schema: bm_schema::CanonicalSpecifier,

	#[serde(flatten)]
	row: RowResult,
}

fn row_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("read a sheet row")
		.description(
			"Read detailed, filterable information from a single sheet row and its related data.",
		)
		.response_with::<200, Json<RowResponse>, _>(|response| {
			response.example(RowResponse {
				schema: bm_schema::CanonicalSpecifier {
					source: "source".into(),
					version: "version".into(),
				},
				row: RowResult::example(1),
			})
		})
}

#[debug_handler(state = RowsState)]
async fn row(
	Path(path): Path<RowPath>,
	State(config): State<LimitConfig>,
	reader: RowReader,
) -> Result<Json<RowResponse>> {
	let row = reader.read_row(
		&path.sheet,
		path.row.row_id,
		path.row.subrow_id,
		config.depth,
	)?;

	Ok(Json(RowResponse {
		schema: reader.schema_specifier,
		row,
	}))
}

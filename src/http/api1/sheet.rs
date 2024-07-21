use std::{collections::HashMap, num::ParseIntError, str::FromStr};

use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	transform::TransformOperation,
};
use anyhow::anyhow;
use axum::{debug_handler, extract::State, Extension, Json};
use either::Either;
use ironworks::{excel, file::exh};
use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject, StringValidation},
	JsonSchema,
};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
	http::service,
	read, schema,
	utility::{anyhow::Anyhow, jsonschema::impl_jsonschema},
};

use super::{
	error::{Error, Result},
	extract::{Path, Query, VersionQuery},
	filter::FilterString,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	list: FilterConfig,
	entry: FilterConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
	depth: u8,
}

#[derive(Debug, Clone, Deserialize)]
struct FilterConfig {
	fields: HashMap<String, FilterString>,
}

pub fn router(config: Config) -> ApiRouter<service::State> {
	ApiRouter::new()
		.api_route("/", get_with(list, list_docs))
		.api_route("/:sheet", get_with(sheet, sheet_docs))
		.api_route("/:sheet/:row", get_with(row, row_docs))
		// Using Extension so I don't need to worry about nested state destructuring.
		.layer(Extension(config))
}

fn list_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list sheets")
		.description("List known excel sheet names that can be read by the API.")
		.response_with::<200, Json<Vec<&'static str>>, _>(|response| {
			response.example(vec!["Action", "Item", "Status"])
		})
}

#[debug_handler(state = service::State)]
async fn list(
	VersionQuery(version_key): VersionQuery,
	State(data): State<service::Data>,
) -> Result<impl IntoApiResponse> {
	let excel = data.version(version_key)?.excel();

	let list = excel.list().anyhow()?;
	let mut names = list
		.iter()
		.map(|name| name.into_owned())
		.collect::<Vec<_>>();
	names.sort();

	Ok(Json(names))
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
	// Data resolution
	/// Language to use for data with no language otherwise specified in the fields filter.
	language: Option<read::LanguageString>,

	/// Schema that row data should be read with.
	schema: Option<schema::Specifier>,

	// Data fields to read for selected rows.
	fields: Option<FilterString>,

	// ID pagination/filtering
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
	schema: schema::CanonicalSpecifier,

	/// Array of rows retrieved by the query.
	rows: Vec<RowResult>,
}

// TODO: ideally this structure is equivalent to the relation metadata from read:: - to the point honestly it probably _should_ be that. yet another thing to consider when reworking read::.
#[derive(Serialize, JsonSchema)]
struct RowResult {
	/// ID of this row.
	row_id: u32,

	/// Subrow ID of this row, when relevant.
	#[serde(skip_serializing_if = "Option::is_none")]
	subrow_id: Option<u16>,

	/// Field values for this row, according to the current schema and field filter.
	fields: ValueString,
	#[serde(skip_serializing_if = "Option::is_none")]
	transient: Option<ValueString>,
}

fn sheet_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list rows in a sheet")
		.description("Read information about one or more rows and their related data.")
		.response_with::<200, Json<SheetResponse>, _>(|response| {
			response.example(SheetResponse {
				schema: schema::CanonicalSpecifier {
					source: "source".into(),
					version: "version".into(),
				},
				rows: vec![row_result_example(1), row_result_example(2)],
			})
		})
}

#[debug_handler(state = service::State)]
async fn sheet(
	Path(path): Path<SheetPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<SheetQuery>,
	State(data): State<service::Data>,
	State(read): State<service::Read>,
	State(schema_provider): State<service::Schema>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoApiResponse> {
	// Resolve arguments with the services.
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(excel::Language::from)
		.unwrap_or_else(|| read.default_language());

	// TODO: Consider extractor for this.
	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;

	// TODO: this should be a utility
	let filter = query
		.fields
		.or_else(|| config.list.fields.get(&schema_specifier.source).cloned())
		.map(|filter_string| filter_string.to_filter(language))
		.ok_or_else(|| {
			Error::Other(anyhow!(
				"missing default list fields for {}",
				schema_specifier.source
			))
		})??;

	let schema = schema_provider.schema(schema_specifier.clone())?;

	// Get a reference to the sheet we'll be reading from.
	// TODO: should this be in super::error as a default extract? minus the sheet specialised case, that is
	let sheet = excel
		.sheet(&path.sheet)
		.map_err(|error| match error {
			ironworks::Error::NotFound(ironworks::ErrorValue::Sheet(..)) => {
				Error::NotFound(error.to_string())
			}
			other => Error::Other(other.into()),
		})?
		.with_default_language(language);

	// Iterate over the sheet, building row results.
	let sheet_iterator = match query.rows {
		// One or more row specifiers were provided, iterate over those specifically.
		Some(specifiers) => Either::Left(specifiers.into_iter()),

		// None were provided, iterate over the sheet itself.
		// TODO: Currently, read:: does _all_ the row fetching itself, which means that we're effectively iterating the sheet here _just_ to get the row IDs, then re-fetching in the read:: code. This... probably isn't too problematic, but worth considering how to approach more betterer. If read:: can be modified to take a row, then the Some() case above can be specailised to the read-row logic and this case can be simplified.
		None => Either::Right(sheet.into_iter().map(|row| RowSpecifier {
			row_id: row.row_id(),
			subrow_id: row.subrow_id(),
		})),
	};

	// Paginate the results.
	let limit = query
		.limit
		.unwrap_or(config.limit.default)
		.min(config.limit.max);
	let sheet_iterator = sheet_iterator
		// TODO: Improve this - introducing an explicit "after" method on a sheet iterator would allow skipping a lot of busywork. As-is, this is fetching every single row's data.
		.skip_while(|specifier| Some(specifier) <= query.after.as_ref())
		.take(limit);

	// Build Results for the targeted rows.
	let sheet_iterator = sheet_iterator.map(|specifier| {
		read_row_result(
			&read,
			&excel,
			schema.as_ref(),
			&path.sheet,
			specifier,
			language,
			&filter,
			config.limit.depth,
		)
	});

	let rows = sheet_iterator.collect::<Result<Vec<_>>>()?;

	let response = SheetResponse {
		schema: schema_specifier,
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

/// Query parameters accepted by the row endpoint.
#[derive(Deserialize, JsonSchema)]
struct RowQuery {
	/// Language to use for data with no language otherwise specified in the fields filter.
	language: Option<read::LanguageString>,

	/// Schema that row data should be read with.
	schema: Option<schema::Specifier>,

	/// Data fields to read for selected rows.
	fields: Option<FilterString>,
}

/// Response structure for the row endpoint.
#[derive(Serialize, JsonSchema)]
struct RowResponse {
	/// The canonical specifier for the schema used in this response.
	#[schemars(with = "String")]
	schema: schema::CanonicalSpecifier,

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
				schema: schema::CanonicalSpecifier {
					source: "source".into(),
					version: "version".into(),
				},
				row: row_result_example(1),
			})
		})
}

fn row_result_example(row_id: u32) -> RowResult {
	RowResult {
		row_id,
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
		transient: None,
	}
}

#[debug_handler(state = service::State)]
async fn row(
	Path(path): Path<RowPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<RowQuery>,
	State(data): State<service::Data>,
	State(read): State<service::Read>,
	State(schema_provider): State<service::Schema>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoApiResponse> {
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(excel::Language::from)
		.unwrap_or_else(|| read.default_language());

	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;

	let filter = query
		.fields
		.or_else(|| config.entry.fields.get(&schema_specifier.source).cloned())
		.map(|filter_string| filter_string.to_filter(language))
		.ok_or_else(|| {
			Error::Other(anyhow!(
				"missing default entry fields for {}",
				schema_specifier.source
			))
		})??;

	let schema = schema_provider.schema(schema_specifier.clone())?;

	let row_result = read_row_result(
		&read,
		&excel,
		schema.as_ref(),
		&path.sheet,
		path.row,
		language,
		&filter,
		config.limit.depth,
	)?;

	let response = RowResponse {
		schema: schema_specifier,
		row: row_result,
	};

	Ok(Json(response))
}

// TODO: This should probably be shared with search.
fn read_row_result(
	read: &service::Read,
	excel: &excel::Excel,
	schema: &dyn ironworks_schema::Schema,
	sheet: &str,
	row: RowSpecifier,
	language: excel::Language,
	filter: &read::Filter,
	depth: u8,
) -> Result<RowResult> {
	let fields = ValueString(
		read.read(
			&excel,
			schema,
			sheet,
			row.row_id,
			row.subrow_id,
			language,
			&filter,
			depth,
		)?,
		language,
	);

	// Try to read a transient
	// TODO: filtering, opt in/out, etc
	let transient = match read.read(
		&excel,
		schema,
		&format!("{}Transient", sheet),
		row.row_id,
		row.subrow_id,
		language,
		&read::Filter::All,
		depth,
	) {
		Ok(value) => Some(ValueString(value, language)),
		Err(read::Error::NotFound(_)) => None,
		Err(error) => Err(error)?,
	};

	// Check the kind of the sheet to determine if we should report a subrow id.
	// TODO: this is theoretically wasteful, though IW will have cached it anyway.
	let result_subrow_id = match excel.sheet(&sheet).anyhow()?.kind().anyhow()? {
		exh::SheetKind::Subrows => Some(row.subrow_id),
		_ => None,
	};

	Ok(RowResult {
		row_id: row.row_id,
		subrow_id: result_subrow_id,
		fields,
		transient,
	})
}

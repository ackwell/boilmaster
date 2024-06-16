use std::{borrow::Cow, collections::HashMap, num::ParseIntError, str::FromStr};

use aide::{
	axum::{
		routing::{get, get_with},
		ApiRouter, IntoApiResponse,
	},
	transform::TransformOperation,
};
use axum::{debug_handler, extract::State, Extension, Json};
use either::Either;
use ironworks::{excel::Language, file::exh};
use schemars::{
	gen::SchemaGenerator,
	schema::{InstanceType, Schema, SchemaObject, StringValidation},
	JsonSchema,
};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{data::LanguageString, http::service, read, schema, utility::anyhow::Anyhow};

use super::{
	error::{Error, Result},
	extract::{Path, Query, VersionQuery},
	filter::FilterString,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	filter: HashMap<String, FilterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct FilterConfig {
	list: Option<FilterString>,
	entry: Option<FilterString>,
}

pub fn router(config: Config) -> ApiRouter<service::State> {
	ApiRouter::new()
		.api_route("/", get_with(list, list_docs))
		.api_route("/:sheet", get(sheet))
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

impl JsonSchema for RowSpecifier {
	fn schema_name() -> String {
		"RowSpecifier".into()
	}

	fn schema_id() -> Cow<'static, str> {
		Cow::Borrowed(concat!(module_path!(), "::RowSpecifier"))
	}

	fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
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
}

/// Query parameters accepted by the sheet endpoint.
#[derive(Deserialize, JsonSchema)]
struct SheetQuery {
	// Data resolution
	// TODO: these two are external types. How do I want to handle that?
	#[schemars(with = "Option<String>")]
	language: Option<LanguageString>,
	#[schemars(with = "Option<String>")]
	schema: Option<schema::Specifier>,
	fields: Option<FilterString>,

	// ID pagination/filtering
	// TODO: rows needs a format or something
	#[serde(default, deserialize_with = "deserialize_rows")]
	#[schemars(with = "Option<String>")]
	rows: Option<Vec<RowSpecifier>>,
	limit: Option<usize>,
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

#[derive(Serialize, JsonSchema)]
struct SheetResponse {
	// TODO: this is an external type - how do I want to handle that?
	#[schemars(with = "String")]
	schema: schema::CanonicalSpecifier,
	rows: Vec<RowResult>,
}

// TODO: ideally this structure is equivalent to the relation metadata from read:: - to the point honestly it probably _should_ be that. yet another thing to consider when reworking read::.
#[derive(Serialize, JsonSchema)]
struct RowResult {
	row_id: u32,

	#[serde(skip_serializing_if = "Option::is_none")]
	subrow_id: Option<u16>,

	fields: ValueString,
}

#[debug_handler(state = service::State)]
async fn sheet(
	Path(path): Path<SheetPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<SheetQuery>,
	State(data): State<service::Data>,
	State(schema_provider): State<service::Schema>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoApiResponse> {
	// Resolve arguments with the services.
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(Language::from)
		.unwrap_or_else(|| data.default_language());

	// TODO: Consider extractor for this.
	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;

	let filter = query
		.fields
		.or_else(|| {
			config
				.filter
				.get(schema_specifier.source())
				.and_then(|filter_config| filter_config.list.clone())
		})
		.map(|filter_string| filter_string.to_filter(language))
		.unwrap_or(Ok(read::Filter::All))?;

	let schema = schema_provider.schema(schema_specifier.clone())?;

	// Get a reference to the sheet we'll be reading from.
	// TODO: should this be in super::error as a default extract? minus the sheet specialised case, that is
	let sheet = excel.sheet(&path.sheet).map_err(|error| match error {
		ironworks::Error::NotFound(ironworks::ErrorValue::Sheet(..)) => {
			Error::NotFound(error.to_string())
		}
		other => Error::Other(other.into()),
	})?;

	// Iterate over the sheet, building row results.
	// TODO: look into changing the row builder in iw so this assignment isn't required - moving to an owned value would also possibly allow me to move this builder into the None case below.
	let mut builder = sheet.with();
	builder.language(language);

	let sheet_iterator = match query.rows {
		// One or more row specifiers were provided, iterate over those specifically.
		Some(specifiers) => Either::Left(specifiers.into_iter()),

		// None were provided, iterate over the sheet itself.
		// TODO: Currently, read:: does _all_ the row fetching itself, which means that we're effectively iterating the sheet here _just_ to get the row IDs, then re-fetching in the read:: code. This... probably isn't too problematic, but worth considering how to approach more betterer. If read:: can be modified to take a row, then the Some() case above can be specailised to the read-row logic and this case can be simplified.
		None => Either::Right(builder.iter().map(|row| RowSpecifier {
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
	let sheet_kind = sheet.kind().anyhow()?;
	let sheet_iterator = sheet_iterator.map(|specifier| {
		let row_id = specifier.row_id;
		let subrow_id = specifier.subrow_id;

		// TODO: This is pretty wasteful to call inside a loop, revisit actual read logic.
		// TODO: at the moment, an unknown row specifier will cause excel to error with a NotFound (which is fine), however read:: then squashes that with anyhow, meaning the error gets hidden in a 500 ISE. revisit error handling in read:: while i'm at it ref. the above.
		let fields = read::read(
			&excel,
			schema.as_ref(),
			&path.sheet,
			row_id,
			subrow_id,
			language,
			&filter,
		)?;

		Ok(RowResult {
			row_id,
			subrow_id: match sheet_kind {
				exh::SheetKind::Subrows => Some(subrow_id),
				_ => None,
			},
			fields: ValueString(fields, language),
		})
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
	#[schemars(with = "Option<String>")]
	language: Option<LanguageString>,
	#[schemars(with = "Option<String>")]
	schema: Option<schema::Specifier>,
	fields: Option<FilterString>,
}

#[derive(Serialize, JsonSchema)]
struct RowResponse {
	#[schemars(with = "String")]
	schema: schema::CanonicalSpecifier,

	#[serde(flatten)]
	row: RowResult,
}

fn row_docs(operation: TransformOperation) -> TransformOperation {
	operation.summary("read a sheet row").description(
		"Read detailed, filterable information from a single sheet row and its related data.",
	)
}

#[debug_handler(state = service::State)]
async fn row(
	Path(path): Path<RowPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<RowQuery>,
	State(data): State<service::Data>,
	State(schema_provider): State<service::Schema>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoApiResponse> {
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(Language::from)
		.unwrap_or_else(|| data.default_language());

	let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;

	let filter = query
		.fields
		.or_else(|| {
			config
				.filter
				.get(schema_specifier.source())
				.and_then(|filter_config| filter_config.entry.clone())
		})
		.map(|filter_string| filter_string.to_filter(language))
		.unwrap_or(Ok(read::Filter::All))?;

	let schema = schema_provider.schema(schema_specifier.clone())?;

	let row_id = path.row.row_id;
	let subrow_id = path.row.subrow_id;

	let fields = read::read(
		&excel,
		schema.as_ref(),
		&path.sheet,
		row_id,
		subrow_id,
		language,
		&filter,
	)?;

	// Check the kind of the sheet to determine if we should report a subrow id.
	// TODO: this is theoretically wasteful, though IW will have cached it anyway.
	let result_subrow_id = match excel.sheet(&path.sheet).anyhow()?.kind().anyhow()? {
		exh::SheetKind::Subrows => Some(subrow_id),
		_ => None,
	};

	let response = RowResponse {
		schema: schema_specifier,
		row: RowResult {
			row_id,
			subrow_id: result_subrow_id,
			fields: ValueString(fields, language),
		},
	};

	Ok(Json(response))
}

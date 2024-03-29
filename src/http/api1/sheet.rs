use std::{collections::HashMap, num::ParseIntError, str::FromStr};

use axum::{
	debug_handler, extract::State, response::IntoResponse, routing::get, Extension, Json, Router,
};
use either::Either;
use ironworks::{excel::Language, file::exh};
use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
	data::LanguageString,
	http::service,
	read, schema,
	utility::{anyhow::Anyhow, warnings::Warnings},
};

use super::{
	error::{Error, Result},
	extract::{Path, Query, VersionQuery},
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit: LimitConfig,

	#[serde(deserialize_with = "deserialize_filter")]
	filter: HashMap<String, Option<read::Filter>>,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitConfig {
	default: usize,
	max: usize,
}

fn deserialize_filter<'de, D>(
	deserializer: D,
) -> Result<HashMap<String, Option<read::Filter>>, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = match Option::<HashMap<String, String>>::deserialize(deserializer)? {
		Some(value) => value,
		None => return Ok(HashMap::new()),
	};

	let filters = raw
		.into_iter()
		.map(|(schema, filter_string)| {
			let filter_parsed = filter_string.parse::<Warnings<Option<read::Filter>>>()?;

			// TODO: is logging going to be a common pattern? should it be pulled into the util?
			let (filter, warnings) = filter_parsed.decompose();
			for warning in warnings {
				tracing::warn!(%schema, %warning, "default filter warning");
			}

			Ok((schema, filter))
		})
		.collect::<Result<_>>()
		.map_err(de::Error::custom)?;

	Ok(filters)
}

pub fn router(config: Config) -> Router<service::State> {
	Router::new()
		.route("/", get(list))
		.route("/:sheet", get(sheet))
		.route("/:sheet/:row", get(row))
		// Using Extension so I don't need to worry about nested state destructuring.
		.layer(Extension(config))
}

#[debug_handler(state = service::State)]
async fn list(
	VersionQuery(version_key): VersionQuery,
	State(data): State<service::Data>,
) -> Result<impl IntoResponse> {
	let excel = data.version(version_key)?.excel();

	let list = excel.list().anyhow()?;
	let mut names = list
		.iter()
		.map(|name| name.into_owned())
		.collect::<Vec<_>>();
	names.sort();

	Ok(Json(names))
}

#[derive(Deserialize)]
struct SheetPath {
	sheet: String,
}

#[derive(Debug)]
struct RowSpecifier {
	row_id: u32,
	subrow_id: Option<u16>,
}

impl FromStr for RowSpecifier {
	type Err = ParseIntError;

	fn from_str(string: &str) -> Result<Self, Self::Err> {
		let out = match string.split_once(':') {
			Some((row_id, subrow_id)) => Self {
				row_id: row_id.parse()?,
				subrow_id: Some(subrow_id.parse()?),
			},
			None => Self {
				row_id: string.parse()?,
				subrow_id: None,
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

#[derive(Deserialize)]
struct SheetQuery {
	// Data resolution
	language: Option<LanguageString>,
	schema: Option<schema::Specifier>,
	// TODO: this is pretty cruddy, rethink this when revisiting read::
	fields: Option<Warnings<Option<read::Filter>>>,

	// ID pagination/filtering
	#[serde(default, deserialize_with = "deserialize_rows")]
	rows: Option<Vec<RowSpecifier>>,
	page: Option<usize>,
	limit: Option<usize>,
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

#[derive(Serialize)]
struct SheetResponse {
	rows: Vec<RowResult>,

	// TODO: maybe this should be provided by serializing `Warnings` itself?
	#[serde(skip_serializing_if = "Vec::is_empty")]
	warnings: Vec<String>,
}

// TODO: ideally this structure is equivalent to the relation metadata from read:: - to the point honestly it probably _should_ be that. yet another thing to consider when reworking read::.
#[derive(Serialize)]
struct RowResult {
	row_id: u32,

	#[serde(skip_serializing_if = "Option::is_none")]
	subrow_id: Option<u16>,

	fields: Option<read::Value>,
}

#[debug_handler(state = service::State)]
async fn sheet(
	Path(path): Path<SheetPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<SheetQuery>,
	State(data): State<service::Data>,
	State(schema_provider): State<service::Schema>,
	Extension(config): Extension<Config>,
) -> Result<impl IntoResponse> {
	// Resolve arguments with the services.
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(Language::from)
		.unwrap_or_else(|| data.default_language());

	// TODO: Consider extractor for this.
	let schema_specifier = schema_provider.canonicalize(query.schema)?;

	let (field_filter, warnings) = query
		.fields
		.unwrap_or_else(|| Warnings::new(None))
		.decompose();

	let field_filter = field_filter.or_else(|| {
		config
			.filter
			.get(schema_specifier.source())
			.cloned()
			.flatten()
	});

	let schema = schema_provider.schema(schema_specifier)?;

	// Get a reference to the sheet we'll be reading from.
	// TODO: should this be in super::error as a default extract? minus the sheet specialised case, that is
	let sheet = excel.sheet(path.sheet).map_err(|error| match error {
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
			subrow_id: Some(row.subrow_id()),
		})),
	};

	// Paginate the results.
	let limit = query
		.limit
		.unwrap_or(config.limit.default)
		.min(config.limit.max);
	let offset = query.page.unwrap_or(0) * limit;
	let sheet_iterator = sheet_iterator.skip(offset).take(limit);

	// Build Results for the targeted rows.
	let sheet_kind = sheet.kind().anyhow()?;
	let sheet_iterator = sheet_iterator.map(|specifier| {
		let row_id = specifier.row_id;
		let subrow_id = specifier.subrow_id.unwrap_or(0);

		// TODO: This is pretty wasteful to call inside a loop, revisit actual read logic.
		// TODO: at the moment, an unknown row specifier will cause excel to error with a NotFound (which is fine), however read:: then squashes that with anyhow, meaning the error gets hidden in a 500 ISE. revisit error handling in read:: while i'm at it ref. the above.
		let fields = read::read(
			&excel,
			schema.as_ref(),
			language,
			field_filter.as_ref(),
			&sheet.name(),
			row_id,
			subrow_id,
		)?;

		Ok(RowResult {
			row_id,
			subrow_id: match sheet_kind {
				exh::SheetKind::Subrows => Some(subrow_id),
				_ => None,
			},
			fields: Some(fields),
		})
	});

	let rows = sheet_iterator.collect::<Result<Vec<_>>>()?;

	let response = SheetResponse { rows, warnings };

	Ok(Json(response))
}

#[derive(Deserialize)]
struct RowPath {
	sheet: String,
	row: RowSpecifier,
}

#[derive(Deserialize)]
struct RowQuery {
	language: Option<LanguageString>,
	schema: Option<schema::Specifier>,
	fields: Option<Warnings<Option<read::Filter>>>,
}

#[derive(Serialize)]
struct RowResponse {
	#[serde(flatten)]
	row: RowResult,

	#[serde(skip_serializing_if = "Vec::is_empty")]
	warnings: Vec<String>,
}

#[debug_handler(state = service::State)]
async fn row(
	Path(path): Path<RowPath>,
	VersionQuery(version_key): VersionQuery,
	Query(query): Query<RowQuery>,
	State(data): State<service::Data>,
	State(schema_provider): State<service::Schema>,
) -> Result<impl IntoResponse> {
	let excel = data.version(version_key)?.excel();

	let language = query
		.language
		.map(Language::from)
		.unwrap_or_else(|| data.default_language());

	let schema_specifier = schema_provider.canonicalize(query.schema)?;
	let schema = schema_provider.schema(schema_specifier)?;

	let (field_filter, warnings) = query
		.fields
		.unwrap_or_else(|| Warnings::new(None))
		.decompose();

	let row_id = path.row.row_id;
	let subrow_id = path.row.subrow_id;

	let fields = read::read(
		&excel,
		schema.as_ref(),
		language,
		field_filter.as_ref(),
		&path.sheet,
		row_id,
		subrow_id.unwrap_or(0),
	)?;

	let response = RowResponse {
		row: RowResult {
			row_id,
			// NOTE: this results in subrow being reported if it's included in path, even on non-subrow sheets (though anything but :0 on those throws an error)
			subrow_id,
			fields: Some(fields),
		},
		warnings,
	};

	Ok(Json(response))
}

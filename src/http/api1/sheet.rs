use axum::{debug_handler, extract::State, response::IntoResponse, routing::get, Json, Router};
use ironworks::{excel::Language, file::exh};
use serde::{Deserialize, Serialize};

use crate::{
	data::LanguageString,
	read, schema,
	utility::{anyhow::Anyhow, warnings::Warnings},
};

use super::{
	error::{Error, Result},
	extract::{Path, Query, VersionQuery},
	service,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
	limit_default: usize,
	limit_max: usize,
}

pub fn router(config: Config) -> Router<service::State> {
	Router::new()
		.route("/", get(list))
		.route("/:sheet", get(sheet))
		// TODO: combine these as `/:sheet/*id` and use a deserialiser thing like the specifier - then i can reuse that deser thing for ids=... too
		.route("/:sheet/:row", get(row))
		.route("/:sheet/:row/:subrow", get(row))
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

#[derive(Deserialize)]
struct SheetQuery {
	// Data resolution
	language: Option<LanguageString>,
	schema: Option<schema::Specifier>,
	// TODO: this is pretty cruddy, rethink this when revisiting read::
	fields: Option<Warnings<Option<read::Filter>>>,

	// ID pagination/filtering
	page: Option<usize>,
	limit: Option<usize>,
}

#[derive(Serialize)]
struct SheetResponse {
	rows: Vec<RowResult>,
}

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
	let schema = schema_provider.schema(query.schema.as_ref())?;

	let (field_filter, warnings) = query
		.fields
		.unwrap_or_else(|| Warnings::new(None))
		.decompose();

	// Get a reference to the sheet we'll be reading from.
	let sheet = excel.sheet(path.sheet).map_err(|error| match error {
		ironworks::Error::NotFound(ironworks::ErrorValue::Sheet(..)) => {
			Error::NotFound(error.to_string())
		}
		other => Error::Other(other.into()),
	})?;

	// Iterate over the sheet, building row results.
	// todo look into changing the row builder in iw so this assignment isn't required
	let mut builder = sheet.with();
	let sheet_iterator = builder.language(language).iter();

	// Paginate the results.
	let limit = query
		.limit
		.unwrap_or(config.limit_default)
		.min(config.limit_max);
	let offset = query.page.unwrap_or(0) * limit;
	let sheet_iterator = sheet_iterator.skip(offset).take(limit);

	// Build Results for the targeted rows.
	let sheet_kind = sheet.kind().anyhow()?;
	let sheet_iterator = sheet_iterator.map(|row| {
		let row_id = row.row_id();
		let subrow_id = row.subrow_id();

		// TODO: This is pretty wasteful to call inside a loop, revisit actual read logic.
		let fields = read::read(
			&excel,
			schema.as_ref(),
			language,
			field_filter.as_ref(),
			&sheet.name(),
			row_id,
			subrow_id,
		)?;

		//
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

	let response = SheetResponse { rows };

	Ok(Json(response))
}

#[derive(Deserialize)]
struct RowPath {
	sheet: String,
	row: u32,
	subrow: Option<u16>,
}

#[debug_handler(state = service::State)]
async fn row(Path(path): Path<RowPath>) -> impl IntoResponse {
	"todo"
}

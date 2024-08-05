use std::{collections::HashMap, sync::Arc};

use aide::OperationIo;
use anyhow::anyhow;
use axum::{
	async_trait,
	extract::{FromRef, FromRequestParts},
	http::request::Parts,
	Extension, RequestPartsExt,
};
use ironworks::{excel, file::exh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{http::service, read, schema, utility::anyhow::Anyhow};

use super::{
	error::{Error, Result},
	extract::{Query, VersionQuery},
	filter::FilterString,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct RowReaderConfig {
	fields: HashMap<String, FilterString>,
	transient: HashMap<String, FilterString>,
}

// todo: maybe it's readrequest? something? "rowreader" is perhaps overindexing, and i should be referring to it simply as "read"?
/// Query parameters accepted by endpoints that retrieve excel row data.
#[derive(Deserialize, JsonSchema)]
struct RowReaderQuery {
	/// Language to use for data with no language otherwise specified in the fields filter.
	language: Option<read::LanguageString>,

	/// Schema that row data should be read with.
	schema: Option<schema::Specifier>,

	/// Data fields to read for selected rows.
	fields: Option<FilterString>,

	/// Data fields to read for selected rows' transient row, if any is present.
	transient: Option<FilterString>,
}

// TODO: ideally this structure is equivalent to the relation metadata from read:: - to the point honestly it probably _should_ be that. yet another thing to consider when reworking read::.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RowResult {
	/// ID of this row.
	pub row_id: u32,

	/// Subrow ID of this row, when relevant.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subrow_id: Option<u16>,

	/// Field values for this row, according to the current schema and field filter.
	pub fields: ValueString,

	/// Field values for this row's transient row, if any is present, according to
	/// the current schema and transient filter.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub transient: Option<ValueString>,
}

impl RowResult {
	pub fn example(row_id: u32) -> RowResult {
		RowResult {
			row_id,
			subrow_id: None,
			fields: ValueString(
				read::Value::Struct(HashMap::from([(
					"FieldName".into(),
					read::Value::Scalar(excel::Field::U32(14)),
				)])),
				excel::Language::English,
			),
			// TODO: should this have an example?
			transient: None,
		}
	}
}

#[derive(OperationIo)]
#[aide(input_with = "Query<RowReaderQuery>")]
pub struct RowReader {
	read: service::Read,
	pub excel: Arc<excel::Excel>,
	pub schema_specifier: schema::CanonicalSpecifier,
	schema: Box<dyn ironworks_schema::Schema + Send>,
	pub language: excel::Language,
	fields: read::Filter,
	transient: Option<read::Filter>,
}

// todo maybe an extra bit of state requirements on this for the filters? that would allow the filters to be wired up per-handler i think. not sure how that aligns with existing state though
#[async_trait]
impl<S> FromRequestParts<S> for RowReader
where
	S: Send + Sync,
	service::Data: FromRef<S>,
	service::Read: FromRef<S>,
	service::Schema: FromRef<S>,
	service::Version: FromRef<S>,
{
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let data = service::Data::from_ref(state);
		let read = service::Read::from_ref(state);
		let schema_provider = service::Schema::from_ref(state);

		let VersionQuery(version_key) = parts.extract_with_state::<VersionQuery, _>(state).await?;
		let Query(query) = parts.extract::<Query<RowReaderQuery>>().await?;

		// TODO: can i at all get this added to state?
		let Extension(config) = parts
			.extract::<Extension<RowReaderConfig>>()
			.await
			.map_err(|error| Error::Other(error.into()))?;

		let excel = data.version(version_key)?.excel();

		// TODO: should this be a bit like versionquery for the schema shit?
		let schema_specifier = schema_provider.canonicalize(query.schema, version_key)?;

		let language = query
			.language
			.map(excel::Language::from)
			.unwrap_or_else(|| read.default_language());

		let fields = query
			.fields
			.or_else(|| config.fields.get(&schema_specifier.source).cloned())
			.ok_or_else(|| anyhow!("missing default fields for {}", schema_specifier.source))?
			.to_filter(language)?;

		let transient_string = query
			.transient
			.or_else(|| config.transient.get(&schema_specifier.source).cloned())
			.ok_or_else(|| anyhow!("missing default transient for {}", schema_specifier.source))?;

		let transient = match transient_string.is_empty() {
			true => None,
			false => Some(transient_string.to_filter(language)?),
		};

		let schema = schema_provider.schema(schema_specifier.clone())?;

		Ok(Self {
			read,
			excel,
			schema_specifier,
			schema,
			language,
			fields,
			transient,
		})
	}
}

impl RowReader {
	// todo: should i move the depth somewhere else? it _is_ effectively static config
	pub fn read_row(
		&self,
		sheet: &str,
		row_id: u32,
		subrow_id: u16,
		depth: u8,
	) -> Result<RowResult> {
		let fields = ValueString(
			self.read.read(
				&self.excel,
				self.schema.as_ref(),
				sheet,
				row_id,
				subrow_id,
				self.language,
				&self.fields,
				depth,
			)?,
			self.language,
		);

		// Try to read a transient row.
		let transient = match self.transient.as_ref() {
			None => None,
			Some(filter) => match self.read.read(
				&self.excel,
				self.schema.as_ref(),
				&format!("{}Transient", sheet),
				row_id,
				subrow_id,
				self.language,
				filter,
				depth,
			) {
				Ok(value) => Some(ValueString(value, self.language)),
				Err(read::Error::NotFound(_)) => None,
				Err(error) => Err(error)?,
			},
		};

		// Check the kind of the sheet to determine if we should report a subrow id.
		// TODO: this is theoretically wasteful, though IW will have cached it anyway.
		let result_subrow_id = match self.excel.sheet(&sheet).anyhow()?.kind().anyhow()? {
			exh::SheetKind::Subrows => Some(subrow_id),
			_ => None,
		};

		Ok(RowResult {
			row_id,
			subrow_id: result_subrow_id,
			fields,
			transient,
		})
	}
}

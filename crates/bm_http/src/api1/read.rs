use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use aide::OperationIo;
use anyhow::anyhow;
use axum::{
	RequestPartsExt,
	extract::{FromRef, FromRequestParts},
	http::request::Parts,
};
use bm_read as read;
use bm_version::VersionKey;
use ironworks::{excel, file::exh, sestring::format::Input};
use schemars::{
	JsonSchema,
	r#gen::SchemaGenerator,
	schema::{InstanceType, Metadata, Schema, SchemaObject, StringValidation},
};
use serde::{Deserialize, Serialize};

use crate::service;

use super::{
	error::{Error, Result},
	extract::{Query, VersionQuery},
	filter::FilterString,
	jsonschema::impl_jsonschema,
	string::build_input,
	value::ValueString,
};

#[derive(Debug, Clone, Deserialize)]
pub struct RowReaderConfig {
	fields: HashMap<String, FilterString>,
	transient: HashMap<String, FilterString>,
}

#[derive(Debug, Default, Clone)]
pub struct RowReaderState {
	string_input: Arc<RwLock<HashMap<VersionKey, Arc<Input>>>>,
}

impl RowReaderState {
	fn input(&self, version: VersionKey, excel: &excel::Excel) -> Result<Arc<Input>> {
		let inputs = self.string_input.read().expect("poisoned");
		if let Some(input) = inputs.get(&version) {
			return Ok(input.clone());
		}

		drop(inputs);
		let mut inputs_mut = self.string_input.write().expect("poisoned");
		let input = Arc::new(build_input(excel)?);
		inputs_mut.insert(version, input.clone());

		Ok(input)
	}
}

// todo: maybe it's readrequest? something? "rowreader" is perhaps overindexing, and i should be referring to it simply as "read"?
/// Query parameters accepted by endpoints that retrieve excel row data.
#[derive(Deserialize, JsonSchema)]
struct RowReaderQuery {
	/// Language to use for data with no language otherwise specified in the fields filter.
	language: Option<SchemaLanguage>,

	/// Schema that row data should be read with.
	schema: Option<SchemaSpecifier>,

	/// Data fields to read for selected rows.
	fields: Option<FilterString>,

	/// Data fields to read for selected rows' transient row, if any is present.
	transient: Option<FilterString>,
}

#[derive(Deserialize)]
#[repr(transparent)]
struct SchemaLanguage(read::LanguageString);

impl_jsonschema!(SchemaLanguage, languagestring_schema);
fn languagestring_schema(_generator: &mut SchemaGenerator) -> Schema {
	// TODO: keep this up to date with the full list. Honestly, this should be iterating the enum in excel or similar.
	let languages = [
		excel::Language::None,
		excel::Language::Japanese,
		excel::Language::English,
		excel::Language::German,
		excel::Language::French,
		excel::Language::ChineseSimplified,
		excel::Language::ChineseTraditional,
		excel::Language::Korean,
	];

	Schema::Object(SchemaObject {
		metadata: Some(
			Metadata {
				description: Some("Known languages supported by the game data format. **NOTE:** Not all languages that are supported by the format are valid for all editions of the game. For example, the global game client acknowledges the existence of `chs` and `kr`, however does not provide any data for them.".into()),
				..Default::default()
			}
			.into(),
		),
		instance_type: Some(InstanceType::String.into()),
		enum_values: Some(
			languages
				.map(|language| read::LanguageString::from(language).to_string().into())
				.to_vec(),
		),
		..Default::default()
	})
}

#[derive(Deserialize)]
#[repr(transparent)]
struct SchemaSpecifier(bm_schema::Specifier);

impl_jsonschema!(SchemaSpecifier, specifier_jsonschema);
fn specifier_jsonschema(_generator: &mut SchemaGenerator) -> Schema {
	Schema::Object(SchemaObject {
		instance_type: Some(InstanceType::String.into()),
		string: Some(
			StringValidation {
				pattern: Some("^.+(@.+)?$".into()),
				..Default::default()
			}
			.into(),
		),
		..Default::default()
	})
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
				Input::new().into(),
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
	pub specifiers: Specifiers,
	schema: Box<dyn ironworks_schema::Schema + Send>,
	pub language: excel::Language,
	fields: read::Filter,
	transient: Option<read::Filter>,
	string_input: Arc<Input>,

	// TODO: Horrifying Hack. I'm not convinced the read module should be "smart" enough to handle cross-iteration caching, but I _do_ need to set up some degree of shared cache within a single row-read, and lifting that up to an iteration point is probably worthwhile so e.g. item0 doesn't get read N times.
	rows_read: u32,
}

#[derive(Serialize, JsonSchema)]
pub struct Specifiers {
	/// The canonical specifier for the schema used in this response.
	#[schemars(with = "String")]
	pub schema: bm_schema::CanonicalSpecifier,

	/// The canonical specifier for the version used in this response.
	#[schemars(with = "String")]
	pub version: bm_version::VersionKey,
}

// todo maybe an extra bit of state requirements on this for the filters? that would allow the filters to be wired up per-handler i think. not sure how that aligns with existing state though
impl<S> FromRequestParts<S> for RowReader
where
	S: Send + Sync,
	service::Service: FromRef<S>,
	RowReaderConfig: FromRef<S>,
	RowReaderState: FromRef<S>,
{
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let VersionQuery(version_key) = parts.extract_with_state::<VersionQuery, _>(state).await?;
		let Query(query) = parts.extract::<Query<RowReaderQuery>>().await?;

		let service::Service {
			data,
			read,
			schema: schema_provider,
			..
		} = service::Service::from_ref(state);
		let config = RowReaderConfig::from_ref(state);
		let state = RowReaderState::from_ref(state);

		let excel = data.version(version_key)?.excel();

		// TODO: should this be a bit like versionquery for the schema shit?
		let schema_specifier =
			schema_provider.canonicalize(query.schema.map(|wrap| wrap.0), version_key)?;

		let language = query
			.language
			.map(|wrap| excel::Language::from(wrap.0))
			.unwrap_or_else(|| read.default_language());

		let string_input = state.input(version_key, &excel)?;

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
			specifiers: Specifiers {
				schema: schema_specifier,
				version: version_key,
			},
			schema,
			language,
			fields,
			transient,
			string_input,

			rows_read: 0,
		})
	}
}

impl RowReader {
	pub fn track_rows_read_and_maybe_explode(&mut self, extra: u32) -> Result<()> {
		// This is a horrific hardcoded hack.
		self.rows_read += extra;
		match self.rows_read > 20_000 {
			true => Err(Error::Invalid("Fulfilling this request would require processing over 20,000 rows of data. Please limit the scope of the fields you are reading. If you're hitting this, consider joining the XIVAPI discord @ discord.gg/MFFVHWC - we may be able to help improve your query.".into())),
			false => Ok(()),
		}
	}

	// todo: should i move the depth somewhere else? it _is_ effectively static config
	pub fn read_row(
		&mut self,
		sheet: &str,
		row_id: u32,
		subrow_id: u16,
		depth: u8,
	) -> Result<RowResult> {
		let (value, rows_read) = self.read.read(
			&self.excel,
			self.schema.as_ref(),
			sheet,
			row_id,
			subrow_id,
			self.language,
			&self.fields,
			depth,
		)?;
		self.track_rows_read_and_maybe_explode(rows_read)?;
		let fields = ValueString(value, self.language, self.string_input.clone());

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
				Ok((value, rows_read)) => {
					self.track_rows_read_and_maybe_explode(rows_read)?;
					Some(ValueString(value, self.language, self.string_input.clone()))
				}
				Err(read::Error::NotFound(_)) => None,
				Err(error) => Err(error)?,
			},
		};

		// Check the kind of the sheet to determine if we should report a subrow id.
		// TODO: this is theoretically wasteful, though IW will have cached it anyway.
		let result_subrow_id = match self.excel.sheet(&sheet)?.kind()? {
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

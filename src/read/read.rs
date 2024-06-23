use std::{
	borrow::Cow,
	collections::{hash_map, HashMap},
	iter,
	ops::Range,
};

use anyhow::{anyhow, Context};
use ironworks::{excel, file::exh};
use ironworks_schema as schema;
use nohash_hasher::IntMap;

use crate::read::Language;

use super::{
	error::{Error, MismatchError, Result},
	filter::Filter,
	value::{Reference, StructKey, Value},
};

pub fn read(
	excel: &excel::Excel,
	schema: &dyn schema::Schema,

	sheet_name: &str,
	row_id: u32,
	subrow_id: u16,

	default_language: excel::Language,

	filter: &Filter,
	depth: u8,
) -> Result<Value> {
	let value = read_sheet(ReaderContext {
		excel,
		schema,

		sheet: sheet_name,
		language: default_language,
		row_id,
		subrow_id,

		filter,
		rows: &mut HashMap::new(),
		columns: &[],
		depth,
	})?;

	Ok(value)
}

fn read_sheet(context: ReaderContext) -> Result<Value> {
	let sheet_name = context.sheet;
	let sheet_data = context.excel.sheet(sheet_name)?;

	// Fabricate an empty schema for missing sheet schemas so we're able to read _something_.
	let sheet_schema = match context.schema.sheet(sheet_name) {
		Err(schema::Error::NotFound(schema::ErrorValue::Sheet(sheet_name))) => Ok(schema::Sheet {
			name: sheet_name,
			order: schema::Order::Offset,
			node: schema::Node::Struct(vec![]),
		}),
		other => other,
	}?;

	let columns = get_sorted_columns(&sheet_schema, &sheet_data)?;

	let value = read_node(
		&sheet_schema.node,
		ReaderContext {
			columns: &columns,

			..context
		},
	)?;

	Ok(value)
}

fn get_sorted_columns(
	schema: &schema::Sheet,
	data: &excel::Sheet<'_, &str>,
) -> Result<Vec<exh::ColumnDefinition>> {
	let mut columns = data.columns()?;

	match schema.order {
		schema::Order::Index => (),
		// NOTE: It's important to maintain the sort order here for PackedBool ordering
		schema::Order::Offset => columns.sort_by_key(|column| column.offset()),
	};

	Ok(columns)
}

fn read_node(node: &schema::Node, context: ReaderContext) -> Result<Value> {
	use schema::Node as N;
	match node {
		N::Array { count, node } => read_node_array(node, *count, context),
		N::Scalar(scalar) => read_node_scalar(scalar, context),
		N::Struct(fields) => read_node_struct(fields, context),
	}
}

fn read_node_scalar(scalar: &schema::Scalar, mut context: ReaderContext) -> Result<Value> {
	let field = context.next_field()?;

	use schema::Scalar as S;
	let out = match scalar {
		S::Default => Value::Scalar(field),
		S::Reference(targets) => read_scalar_reference(field, targets, context)?,
		S::Icon => read_scalar_icon(field)?,

		kind => {
			tracing::warn!(?kind, "unhandled scalar sub-kind");
			Value::Scalar(field)
		}
	};

	Ok(out)
}

fn read_scalar_reference(
	field: excel::Field,
	targets: &[schema::ReferenceTarget],
	context: ReaderContext,
) -> Result<Value> {
	// TODO: are references _always_ i32? like, always always?
	let target_value = convert_reference_value(field)?;

	let mut reference = Reference::Scalar(target_value);

	// A target less than 0 (typically -1) is usually used to signify that a link
	// is not present on this row. Also ensure that we've not run out of recursion
	// depth. We avoid early return if following an active reference chain.
	// TODO: would be neat to halt recursion later, but target checking does have a cost that needs to be considered.
	if target_value < 0 || (context.depth == 0 && context.filter == &Filter::All) {
		return Ok(Value::Reference(reference));
	}
	let target_value = u32::try_from(target_value)
		.expect("target value should always be >= 0 due to prior condition");

	// NOTE: a lot of the TODOs here are immediately break;ing - this is to avoid a potentially correct target that is simply unhandled being ignored and a later, incorrect target being picked as a result.
	for target in targets {
		if let Some(condition) = &target.condition {
			// TODO: This is effectively spinning an entirely new read tree just to check the condition, which is dumb. It'll technically hit cache all the way down, but this is incredibly dumb.
			let mut language_map = IntMap::default();
			language_map.insert(Language(context.language), Filter::All);
			let data = read_sheet(ReaderContext {
				filter: &Filter::Struct(HashMap::from([(
					condition.selector.clone(),
					language_map,
				)])),
				rows: &mut *context.rows,
				..context
			})?;

			let struct_value = match data {
				Value::Struct(mut map) => map
					.remove(&StructKey {
						name: condition.selector.clone(),
						language: context.language,
					})
					.ok_or_else(|| Error::Failure(anyhow!("Schema target condition mismatch.")))?,
				_ => Err(anyhow!(
					"Did not recieve a struct from target condition lookup."
				))?,
			};
			let scalar_value = match struct_value {
				Value::Scalar(field) => read_scalar_u32(field)?,
				_ => Err(anyhow!(
					"Did not recieve a scalar from struct in target condition lookup."
				))?,
			};

			if scalar_value != condition.value {
				continue;
			}
		}

		// TODO: handle retargeted refs
		if target.selector.is_some() {
			tracing::warn!("unhandled target selector: {target:?}");
			break;
		}

		let sheet_data = context.excel.sheet(&target.sheet)?;

		// TODO: handle references targeting subrows (how?)
		if sheet_data.kind()? == exh::SheetKind::Subrows {
			tracing::warn!("unhandled subrow sheet target: {target:?}");
			break;
		}

		// Try to fetch the row data - if no matching row exists, continue to the next target.
		// TODO: handle target selectors
		let row_data = match sheet_data
			.with()
			.language(context.language)
			.row(target_value)
		{
			Err(ironworks::Error::NotFound(ironworks::ErrorValue::Row { .. })) => continue,
			other => other,
		}?;

		let row_id = row_data.row_id();
		let subrow_id = row_data.subrow_id();

		let child_data = read_sheet(ReaderContext {
			sheet: &target.sheet,
			row_id,
			subrow_id,

			rows: &mut HashMap::from([(context.language, row_data)]),
			depth: context.depth.max(1) - 1,

			..context
		})?;

		reference = Reference::Populated {
			value: target_value,
			sheet: target.sheet.to_string(),
			row_id,
			fields: child_data.into(),
		}
	}

	Ok(Value::Reference(reference))
}

fn convert_reference_value(field: excel::Field) -> Result<i32> {
	use excel::Field as F;
	let result = match field {
		F::I8(value) => i32::from(value),
		F::I16(value) => i32::from(value),
		F::I32(value) => value,
		F::I64(value) => value.try_into()?,
		F::U8(value) => i32::from(value),
		F::U16(value) => i32::from(value),
		F::U32(value) => value.try_into()?,
		F::U64(value) => value.try_into()?,

		other => Err(anyhow!("invalid index type {other:?}"))?,
	};
	Ok(result)
}

fn read_scalar_icon(field: excel::Field) -> Result<Value> {
	Ok(Value::Icon(read_scalar_u32(field)?))
}

fn read_scalar_u32(field: excel::Field) -> Result<u32> {
	// TODO: this is getting dumb.
	use excel::Field as F;
	let result = match field {
		F::I8(value) => u32::try_from(value)?,
		F::I16(value) => u32::try_from(value)?,
		F::I32(value) => u32::try_from(value)?,
		F::I64(value) => u32::try_from(value)?,
		F::U8(value) => u32::from(value),
		F::U16(value) => u32::from(value),
		F::U32(value) => value,
		F::U64(value) => u32::try_from(value)?,

		other => Err(anyhow!("invalid u32 type {other:?}"))?,
	};
	Ok(result)
}

fn read_node_array(
	element_node: &schema::Node,
	count: u32,
	mut context: ReaderContext,
) -> Result<Value> {
	let filter = match context.filter {
		Filter::All => &Filter::All,
		Filter::Array(inner) => inner.as_ref(),
		other => {
			return Err(Error::FilterSchemaMismatch(
				context.mismatch_error(format!("expected array filter, got {other:?}")),
			));
		}
	};

	let size = usize::try_from(element_node.size()).context("schema node too large")?;
	let values = (0..count)
		.scan(0usize, |index, _| {
			let Some(columns) = context.columns.get(*index..*index + size) else {
				return Some(Err(Error::SchemaGameMismatch(
					context.mismatch_error(format!("insufficient columns to satisfy array")),
				)));
			};
			*index += size;

			let result = read_node(
				element_node,
				ReaderContext {
					filter,
					columns,
					rows: &mut context.rows,

					..context
				},
			);

			Some(result)
		})
		.collect::<Result<Vec<_>>>()?;

	Ok(Value::Array(values))
}

fn read_node_struct(fields: &[schema::StructField], mut context: ReaderContext) -> Result<Value> {
	let filter_fields = match context.filter {
		Filter::All => None,
		Filter::Struct(filter_fields) => Some(filter_fields),
		other => {
			return Err(Error::FilterSchemaMismatch(
				context.mismatch_error(format!("expected struct filter, got {other:?}")),
			))
		}
	};

	let mut value_fields = HashMap::new();

	for (name, node, columns) in iterate_struct_fields(fields, context.columns)? {
		let language_filters = match filter_fields {
			Some(fields) => either::Left(match fields.get(name.as_ref()) {
				// Filter exists, but has no entry for this name - no languages to filter to.
				None => either::Left(iter::empty()),

				// Entry exists for the name, map the language pairs to the expected shape.
				Some(languages) => either::Right(
					languages
						.iter()
						.map(|(language, filter)| (language.0, filter)),
				),
			}),

			// ::All filter, walk with the current context language.
			None => either::Right(std::iter::once((context.language, &Filter::All))),
		};

		for (language, filter) in language_filters {
			let value = read_node(
				node,
				ReaderContext {
					filter,
					language,
					columns,
					rows: &mut context.rows,
					..context
				},
			)?;

			match value_fields.entry(StructKey {
				name: name.to_string(),
				language,
			}) {
				hash_map::Entry::Vacant(entry) => {
					entry.insert(value);
				}
				hash_map::Entry::Occupied(entry) => {
					tracing::warn!(key = ?entry.key(), "struct key collision");
				}
			}
		}
	}

	// TODO: i can catch filterschemamismatch at the struct level and skip the key - ideally raise a warning in future
	// what about schemagamemismatch?

	Ok(Value::Struct(value_fields))
}

// TODO: this is fairly gnarly - look into a crate for generators, i.e. genawaiter?
fn iterate_struct_fields<'s, 'c>(
	fields: &'s [schema::StructField],
	columns: &'c [exh::ColumnDefinition],
) -> Result<impl Iterator<Item = (Cow<'s, str>, &'s schema::Node, &'c [exh::ColumnDefinition])>> {
	// Eagerly ensure that we have enough columns available to satisfy the struct field definitions.
	let fields_length = match fields.last() {
		Some(field) => {
			usize::try_from(field.offset + field.node.size()).expect("schema field size too large")
		}
		None => 0,
	};

	if fields_length > columns.len() {
		// TODO: use context for the mismatch error?
		return Err(Error::SchemaGameMismatch(MismatchError {
			field: "TODO".into(),
			reason: "not enough columns to satisfy struct".into(),
		}));
	}

	// Utility to generate items for columns not covered by a field.
	let generate_unknowns = |range: Range<usize>| {
		range.map(|offset| {
			let column = &columns[offset];
			(
				Cow::<str>::Owned(format!(
					"unknown{}{}",
					column.offset(),
					unknown_suffix(column.kind())
				)),
				&schema::Node::Scalar(schema::Scalar::Default),
				&columns[offset..offset + 1],
			)
		})
	};

	let items = fields
		.into_iter()
		.scan(0usize, move |last_offset, field| {
			let field_offset =
				usize::try_from(field.offset).expect("schema field offset too large");
			let field_size =
				usize::try_from(field.node.size()).expect("schema field size too large");

			// Generate unknowns for any columns between the last field and this one.
			let items = generate_unknowns(*last_offset..field_offset)
				// Add an item for this field's schema structure.
				.chain(iter::once((
					Cow::<str>::Borrowed(&field.name),
					&field.node,
					&columns[field_offset..field_offset + field_size],
				)));

			*last_offset = field_offset + field_size;

			Some(items)
		})
		.flatten()
		// Generate unkowns for any trailing columns after the last field.
		.chain(generate_unknowns(fields_length..columns.len()));

	Ok(items)
}

fn unknown_suffix(kind: exh::ColumnKind) -> &'static str {
	use exh::ColumnKind as CK;
	match kind {
		CK::PackedBool0 => "_0",
		CK::PackedBool1 => "_1",
		CK::PackedBool2 => "_2",
		CK::PackedBool3 => "_3",
		CK::PackedBool4 => "_4",
		CK::PackedBool5 => "_5",
		CK::PackedBool6 => "_6",
		CK::PackedBool7 => "_7",
		_ => "",
	}
}

struct ReaderContext<'a> {
	excel: &'a excel::Excel<'a>,
	schema: &'a dyn schema::Schema,

	sheet: &'a str,
	language: excel::Language,
	row_id: u32,
	subrow_id: u16,

	filter: &'a Filter,
	columns: &'a [exh::ColumnDefinition],
	rows: &'a mut HashMap<excel::Language, excel::Row>,
	depth: u8,
}

impl ReaderContext<'_> {
	fn next_field(&mut self) -> Result<excel::Field> {
		let column = self.columns.get(0).ok_or_else(|| {
			Error::SchemaGameMismatch(
				self.mismatch_error("tried to read field but no columns available".to_string()),
			)
		})?;

		let row = match self.rows.entry(self.language) {
			hash_map::Entry::Occupied(entry) => entry.into_mut(),
			hash_map::Entry::Vacant(entry) => entry.insert(
				self.excel
					.sheet(self.sheet)?
					.with()
					.language(self.language)
					.subrow(self.row_id, self.subrow_id)?,
			),
		};

		Ok(row.field(column)?)
	}

	fn mismatch_error(&self, reason: impl ToString) -> MismatchError {
		MismatchError {
			field: "TODO: contextual filter path".into(),
			reason: reason.to_string(),
		}
	}
}

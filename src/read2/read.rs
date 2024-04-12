use std::{
	borrow::Cow,
	collections::{hash_map, HashMap},
	iter,
	ops::Range,
};

use anyhow::Context;
use ironworks::{excel, file::exh};
use ironworks_schema as schema;

use crate::read2::error::MismatchError;

use super::{
	error::{Error, Result},
	filter::Filter,
	value::{StructKey, Value},
};

pub fn read(
	excel: &excel::Excel,
	schema: &dyn schema::Schema,

	sheet_name: &str,
	row_id: u32,
	subrow_id: u16,

	default_language: excel::Language,

	filter: &Filter,
) -> Result<Value> {
	let sheet_schema = schema.sheet(sheet_name)?;
	let sheet_data = excel.sheet(sheet_name)?;
	let columns = get_sorted_columns(&sheet_schema, &sheet_data)?;

	let value = read_node(
		&sheet_schema.node,
		ReaderContext {
			excel,

			sheet: sheet_name,
			language: default_language,
			row_id,
			subrow_id,

			filter,
			rows: &mut HashMap::new(),
			columns: &columns,
		},
	)?;

	Ok(value)
}

fn get_sorted_columns(
	schema: &schema::Sheet,
	data: &excel::Sheet<'_, &str>,
) -> Result<Vec<exh::ColumnDefinition>> {
	if schema.order != schema::Order::Index {
		todo!("sheet schema {:?} order", schema.order);
	}

	Ok(data.columns()?)
}

fn read_node(node: &schema::Node, context: ReaderContext) -> Result<Value> {
	use schema::Node as N;
	match node {
		N::Array { count, node } => read_node_array(node, *count, context),
		N::Reference(targets) => read_node_reference(context),
		N::Scalar => read_node_scalar(context),
		N::Struct(fields) => read_node_struct(fields, context),
	}
}

fn read_node_scalar(mut context: ReaderContext) -> Result<Value> {
	context.next_field().map(Value::Scalar)
}

fn read_node_reference(context: ReaderContext) -> Result<Value> {
	// TODO: Implement references
	read_node_scalar(context)
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
				return Some(Err(Error::SchemaGameMismatch(context.mismatch_error(format!("insufficient columns to satisfy array")))));
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

			value_fields.insert(
				StructKey {
					name: name.to_string(),
					language,
				},
				value,
			);
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
	let last_field = &fields[fields.len() - 1];
	let fields_length = usize::try_from(last_field.offset + last_field.node.size())
		.expect("schema field size too large");

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
			(
				Cow::<str>::Owned(format!("unknown{offset}")),
				&schema::Node::Scalar,
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

#[derive(Debug)]
struct ReaderContext<'a> {
	excel: &'a excel::Excel<'a>,

	sheet: &'a str,
	language: excel::Language,
	row_id: u32,
	subrow_id: u16,

	filter: &'a Filter,
	columns: &'a [exh::ColumnDefinition],
	rows: &'a mut HashMap<excel::Language, excel::Row>,
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

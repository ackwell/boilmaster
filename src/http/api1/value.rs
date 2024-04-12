use std::collections::HashMap;

use ironworks::excel;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, SerializeStruct};

use crate::{data, read2};

#[derive(Debug)]
pub struct ValueString(pub read2::Value);
impl Serialize for ValueString {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serialize_value(serializer, &self.0)
	}
}

struct ValueReference<'a>(&'a read2::Value);
impl Serialize for ValueReference<'_> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serialize_value(serializer, self.0)
	}
}

fn serialize_value<S>(serializer: S, value: &read2::Value) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	use read2::Value as V;
	match value {
		V::Array(values) => serialize_array(serializer, values),
		V::Reference(reference) => serialize_reference(serializer, reference),
		V::Scalar(field) => serialize_scalar(serializer, field),
		V::Struct(fields) => serialize_struct(serializer, fields),
	}
}

fn serialize_array<S>(serializer: S, values: &[read2::Value]) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let mut sequence = serializer.serialize_seq(Some(values.len()))?;
	for value in values {
		sequence.serialize_element(&ValueReference(value))?;
	}
	sequence.end()
}

fn serialize_reference<S>(serializer: S, reference: &read2::Reference) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let mut state = serializer.serialize_struct("Reference", 3)?;
	state.serialize_field("value", &reference.value)?;
	match &reference.sheet {
		Some(value) => state.serialize_field("sheet", value)?,
		None => state.skip_field("sheet")?,
	};
	match &reference.fields {
		Some(fields) => state.serialize_field("fields", &ValueReference(fields))?,
		None => state.skip_field("fields")?,
	};
	state.end()
}

fn serialize_scalar<S>(serializer: S, field: &excel::Field) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	use excel::Field as F;
	match field {
		// TODO: more comprehensive sestring handling
		F::String(se_string) => serializer.serialize_str(&se_string.to_string()),
		F::Bool(value) => serializer.serialize_bool(*value),
		F::I8(value) => serializer.serialize_i8(*value),
		F::I16(value) => serializer.serialize_i16(*value),
		F::I32(value) => serializer.serialize_i32(*value),
		F::I64(value) => serializer.serialize_i64(*value),
		F::U8(value) => serializer.serialize_u8(*value),
		F::U16(value) => serializer.serialize_u16(*value),
		F::U32(value) => serializer.serialize_u32(*value),
		F::U64(value) => serializer.serialize_u64(*value),
		F::F32(value) => serializer.serialize_f32(*value),
	}
}

fn serialize_struct<S>(
	serializer: S,
	fields: &HashMap<read2::StructKey, read2::Value>,
) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let mut fields = fields
		.into_iter()
		.map(|(read2::StructKey { name, language }, value)| {
			// TODO: omit language if it's equal to request language
			(
				format!("{name}@{}", data::LanguageString::from(*language)),
				value,
			)
		})
		.collect::<Vec<_>>();

	fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));

	let mut map = serializer.serialize_map(Some(fields.len()))?;
	for (name, value) in fields {
		map.serialize_entry(&name, &ValueReference(value))?;
	}
	map.end()
}

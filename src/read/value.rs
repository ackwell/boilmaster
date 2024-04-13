use std::collections::HashMap;

use ironworks::excel;

#[derive(Debug)]
pub enum Value {
	Array(Vec<Value>),
	Reference(Reference),
	Scalar(excel::Field),
	Struct(HashMap<StructKey, Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructKey {
	pub name: String,
	pub language: excel::Language,
}

#[derive(Debug)]
pub enum Reference {
	Scalar(i32),
	Populated {
		value: u32,
		sheet: String,
		row_id: u32,
		subrow_id: Option<u16>,
		fields: Box<Value>,
	},
}

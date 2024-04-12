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
pub struct Reference {
	pub value: i32,

	pub sheet: Option<String>,
	pub fields: Option<Box<Value>>,
}

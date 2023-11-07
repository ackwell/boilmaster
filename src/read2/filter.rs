use std::collections::HashMap;

use ironworks::excel;

#[derive(Debug, Clone)]
pub enum Filter {
	Struct(HashMap<StructKey, Filter>),
	Array(Box<Filter>),
	All,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructKey {
	pub name: String,
	pub language: Option<excel::Language>,
}


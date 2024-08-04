use std::collections::HashMap;

use ironworks::excel;

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
	Struct(HashMap<String, StructEntry>),
	Array(Box<Filter>),
	All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructEntry {
	pub field: String,
	pub language: excel::Language,
	pub filter: Filter,
}

use std::collections::HashMap;

use ironworks::excel;
use nohash_hasher::IsEnabled;

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
	Struct(HashMap<String, StructEntry>),
	Array(Box<Filter>),
	All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructEntry {
	pub field: String,
	pub language: Language,
	pub filter: Filter,
}

// TODO: Merge with LanguageString?
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Language(pub excel::Language);
impl IsEnabled for Language {}

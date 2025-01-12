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
	pub read_as: As,
	pub filter: Filter,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum As {
	Default,
	Raw,
	// NOTE: Passing this through read (and presumably json in future) really
	// kinda reeks, but the alternative is having api1 store html/json state in
	// some tree other than a filter while it gets read, which also kinda sucks.
	// Would need some intermediary format.
	Html,
}

use std::collections::HashMap;

use ironworks::excel;
use nohash_hasher::{IntMap, IsEnabled};

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
	Struct(HashMap<String, IntMap<Language, Filter>>),
	Array(Box<Filter>),
	All,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Language(pub excel::Language);
impl IsEnabled for Language {}

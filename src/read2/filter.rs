use ironworks::excel;

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
	Struct(Vec<(StructKey, Filter)>),
	Array(Box<Filter>),
	All,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructKey {
	pub name: String,
	pub language: Option<excel::Language>,
}


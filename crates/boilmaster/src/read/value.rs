use std::collections::HashMap;

use ironworks::{excel, sestring::SeString};

#[derive(Debug)]
pub enum Value {
	Array(Vec<Value>),
	// TODO: consider moving icon/html (maybe reference?) into a seperate scalar type/enum (if html is kept)
	Html(SeString<'static>),
	Icon(i32),
	Reference(Reference),
	Scalar(excel::Field),
	Struct(HashMap<String, Value>),
}

#[derive(Debug)]
pub enum Reference {
	Scalar(i32),
	Populated {
		value: u32,
		sheet: String,
		row_id: u32,
		fields: Box<Value>,
	},
}

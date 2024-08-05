use std::collections::HashMap;

use ironworks::excel;

#[derive(Debug)]
pub enum Value {
	Array(Vec<Value>),
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

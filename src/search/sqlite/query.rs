use ironworks::{
	excel::{Field, Language, Row, Sheet},
	file::exh,
};
use sea_query::{
	Alias, ColumnDef, ColumnType, Condition, DynIden, Expr, Iden, InsertStatement, IntoCondition,
	Query, SelectStatement, SimpleExpr, Table, TableCreateStatement, TableDropStatement,
};

use crate::{
	data::LanguageString,
	search::{error::Result, internal_query::post},
};

#[derive(Iden)]
enum KnownColumn {
	RowId,
	SubrowId,
}

pub fn table_create(sheet: &Sheet<String>, language: Language) -> Result<TableCreateStatement> {
	let kind = sheet.kind()?;

	// NOTE: Opting against a WITHOUT ROWID table for these - the benefits they
	// confer aren't particularly meaningful for our workload.
	let mut table = Table::create();
	table
		.table(table_name(sheet, language))
		.col(ColumnDef::new(KnownColumn::RowId).integer().primary_key());

	if matches!(kind, exh::SheetKind::Subrows) {
		table.col(ColumnDef::new(KnownColumn::SubrowId).integer());
	}

	for column in sheet.columns()? {
		table.col(&mut ColumnDef::new_with_type(
			column_name(&column),
			column_type(&column),
		));
	}

	Ok(table.take())
}

pub fn table_drop(sheet: &Sheet<String>, language: Language) -> TableDropStatement {
	Table::drop()
		.table(table_name(&sheet, language))
		.if_exists()
		.take()
}

pub fn table_insert(sheet: &Sheet<String>, language: Language) -> Result<InsertStatement> {
	let kind = sheet.kind()?;

	let mut columns = vec![DynIden::new(KnownColumn::RowId)];

	if matches!(kind, exh::SheetKind::Subrows) {
		columns.push(DynIden::new(KnownColumn::SubrowId));
	}

	for column in sheet.columns()? {
		columns.push(DynIden::new(column_name(&column)));
	}

	let statement = Query::insert()
		.into_table(table_name(sheet, language))
		.columns(columns)
		.to_owned();

	Ok(statement)
}

fn table_name(sheet: &Sheet<String>, language: Language) -> Alias {
	let language_string = LanguageString::from(language);
	Alias::new(format!("sheet-{}@{language_string}", sheet.name()))
}

// TODO: update IW to return an iterator over col defs so this cols param isn't required for shared access
pub fn row_values<'a>(
	sheet: &Sheet<String>,
	row: &Row,
	columns: impl Iterator<Item = &'a exh::ColumnDefinition>,
) -> Result<impl IntoIterator<Item = SimpleExpr>> {
	let kind = sheet.kind()?;

	let mut values: Vec<SimpleExpr> = vec![row.row_id().into()];

	if matches!(kind, exh::SheetKind::Subrows) {
		values.push(row.subrow_id().into());
	}

	for column in columns {
		let field = row.field(column)?;
		values.push(field_value(field));
	}

	Ok(values)
}

fn column_type(column: &exh::ColumnDefinition) -> ColumnType {
	use exh::ColumnKind as CK;
	match column.kind() {
		// Using text for this because we have absolutely no idea how large any given string is going to be.
		CK::String => ColumnType::Text,

		// Pretty much all of this will collapse to "INTEGER" on sqlite but hey. Accuracy.
		CK::Int8 => ColumnType::TinyInteger,
		CK::UInt8 => ColumnType::TinyUnsigned,
		CK::Int16 => ColumnType::SmallInteger,
		CK::UInt16 => ColumnType::SmallUnsigned,
		CK::Int32 => ColumnType::Integer,
		CK::UInt32 => ColumnType::Unsigned,
		CK::Int64 => ColumnType::BigInteger,
		CK::UInt64 => ColumnType::BigUnsigned,
		CK::Float32 => ColumnType::Float,

		CK::Bool
		| CK::PackedBool0
		| CK::PackedBool1
		| CK::PackedBool2
		| CK::PackedBool3
		| CK::PackedBool4
		| CK::PackedBool5
		| CK::PackedBool6
		| CK::PackedBool7 => ColumnType::Boolean,
	}
}

fn column_name(column: &exh::ColumnDefinition) -> Alias {
	let offset = column.offset();

	// For packed bool columns, offset alone is not enough to disambiguate a
	// field - add a suffix of the packed bit position.
	use exh::ColumnKind as CK;
	let suffix = match column.kind() {
		CK::PackedBool0 => "_0",
		CK::PackedBool1 => "_1",
		CK::PackedBool2 => "_2",
		CK::PackedBool3 => "_3",
		CK::PackedBool4 => "_4",
		CK::PackedBool5 => "_5",
		CK::PackedBool6 => "_6",
		CK::PackedBool7 => "_7",
		_ => "",
	};

	Alias::new(format!("{offset}{suffix}"))
}

fn field_value(field: Field) -> SimpleExpr {
	use Field as F;
	match field {
		F::String(sestring) => sestring.to_string().into(),
		F::Bool(value) => value.into(),
		F::I8(value) => value.into(),
		F::I16(value) => value.into(),
		F::I32(value) => value.into(),
		F::I64(value) => value.into(),
		F::U8(value) => value.into(),
		F::U16(value) => value.into(),
		F::U32(value) => value.into(),
		F::U64(value) => value.into(),
		F::F32(value) => value.into(),
	}
}

// ---

pub fn resolve_queries(queries: Vec<(String, post::Node)>) -> SelectStatement {
	// temp. obviously.
	let q1 = queries.into_iter().next().unwrap();
	resolve_query(q1.1)
}

fn resolve_query(node: post::Node) -> SelectStatement {
	let mut query = Query::select();
	query.from(Alias::new("BOGUSTEST"));

	let condition = resolve_node(node);
	query.cond_where(condition);

	query.take()
}

fn resolve_node(node: post::Node) -> Condition {
	// Query::select().cond_where(condition)
	// Condition::;
	match node {
		post::Node::Group(group) => resolve_group(group),
		post::Node::Leaf(leaf) => resolve_leaf(leaf),
	}
}

fn resolve_group(group: post::Group) -> Condition {
	// for a given group, MUST are top-level AND, SHOULD are "AND (A OR B OR C...)", and MUSTNOT are AND NOT
	// given that, the root of a group is all ANDs, so we can use ::all and collect ORs for the SHOULD
	let mut condition = Condition::all();
	let mut shoulds = Condition::any();
	for (occur, node) in group.clauses {
		let inner_condition = resolve_node(node);
		match occur {
			post::Occur::Must => condition = condition.add(inner_condition),
			post::Occur::Should => shoulds = shoulds.add(inner_condition),
			// todo: is this correct?
			post::Occur::MustNot => condition = condition.add(inner_condition.not()),
		}
	}
	// NOTE: we're only adding if c.len=0 here because any number of SHOULDs do not effect the _filtering_ of a query if there's 1 or more MUSTs - only the scoring. which i don't have any idea how to do. well, that's a lie. but still.
	if shoulds.len() > 0 && condition.len() == 0 {
		condition = condition.add(shoulds)
	}

	// TODO: this would need to record a condition for scoring as well
	// realistically; MUSTs in queries will always match, so a scoring structure only needs to account for the SHOULDs as actual conditions, and can pass up a static integer of the number of MUSTs that can be added to the score

	condition
}

fn resolve_leaf(leaf: post::Leaf) -> Condition {
	// TODO: this is obviously bollocks - needs a table/alias reference, and handle language
	let (column_definition, language) = leaf.field;
	let expression = Expr::col(column_name(&column_definition));

	let se = match leaf.operation {
		post::Operation::Relation(relation) => todo!(),
		// TODO: need to handle escaping
		post::Operation::Match(string) => expression.like(format!("%{string}%")),
		post::Operation::Equal(value) => todo!(),
	};

	se.into_condition()
}

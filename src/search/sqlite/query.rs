use std::collections::HashSet;

use ironworks::{
	excel::{Field, Language, Row, Sheet},
	file::exh,
};
use sea_query::{
	Alias, ColumnDef, ColumnType, Condition, DynIden, Expr, Iden, InsertStatement, IntoCondition,
	Query, SelectStatement, SimpleExpr, Table, TableCreateStatement, TableDropStatement, TableRef,
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
		.table(table_name(&sheet.name(), language))
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
		.table(table_name(&sheet.name(), language))
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
		.into_table(table_name(&sheet.name(), language))
		.columns(columns)
		.to_owned();

	Ok(statement)
}

fn table_name(sheet_name: &str, language: Language) -> Alias {
	let language_string = LanguageString::from(language);
	Alias::new(format!("sheet-{sheet_name}@{language_string}"))
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
	let (sheet_name, node) = queries.into_iter().next().unwrap();
	resolve_query(sheet_name, node)
}

fn resolve_query(sheet_name: String, node: post::Node) -> SelectStatement {
	let alias = "alias-0".to_string();

	let ResolveResult {
		condition,
		languages,
	} = resolve_node(
		node,
		&ResolveContext {
			alias: alias.clone(),
		},
	);

	let mut query = Query::select();

	// TODO: this will probably need to be split out for reuse at relation boundaries
	let mut table_references = languages.into_iter().map(|language| {
		let alias = table_alias(&alias, language);
		(
			alias.clone(),
			TableRef::TableAlias(
				DynIden::new(table_name(&sheet_name, language)),
				DynIden::new(alias),
			),
		)
	});

	// TODO: is it possible for there to be no languages and hence no joins? is that a failure? what about on relation boundaries
	let (base_alias, base_reference) = table_references.next().expect("TODO: handle");
	query.from(base_reference);
	for (join_alias, join_reference) in table_references {
		query.inner_join(
			join_reference,
			Expr::col((join_alias, KnownColumn::RowId))
				.equals((base_alias.clone(), KnownColumn::RowId)),
		);
	}

	query.cond_where(condition);

	query.take()
}

struct ResolveContext {
	alias: String,
}

struct ResolveResult {
	condition: Condition,
	languages: HashSet<Language>,
}

fn resolve_node(node: post::Node, context: &ResolveContext) -> ResolveResult {
	match node {
		post::Node::Group(group) => resolve_group(group, context),
		post::Node::Leaf(leaf) => resolve_leaf(leaf, context),
	}
}

fn resolve_group(group: post::Group, context: &ResolveContext) -> ResolveResult {
	// for a given group, MUST are top-level AND, SHOULD are "AND (A OR B OR C...)", and MUSTNOT are AND NOT
	// given that, the root of a group is all ANDs, so we can use ::all and collect ORs for the SHOULD
	let mut must = Condition::all();
	let mut should = Condition::any();
	let mut must_not = Condition::any().not();

	let mut languages = HashSet::new();

	for (occur, node) in group.clauses {
		let ResolveResult {
			condition: inner_condition,
			languages: inner_languages,
		} = resolve_node(node, context);

		match occur {
			post::Occur::Must => must = must.add(inner_condition),
			post::Occur::Should => should = should.add(inner_condition),
			post::Occur::MustNot => must_not = must_not.add(inner_condition),
		}

		languages.extend(inner_languages)
	}

	// NOTE: we're only adding if c.len=0 here because any number of SHOULDs do not effect the _filtering_ of a query if there's 1 or more MUSTs - only the scoring. which i don't have any idea how to do. well, that's a lie. but still.
	if should.len() > 0 && must.len() == 0 {
		must = must.add(should)
	}

	if must_not.len() > 0 {
		must = must.add(must_not)
	}

	// TODO: this would need to record a condition for scoring as well
	// realistically; MUSTs in queries will always match, so a scoring structure only needs to account for the SHOULDs as actual conditions, and can pass up a static integer of the number of MUSTs that can be added to the score

	ResolveResult {
		condition: must,
		languages,
	}
}

fn resolve_leaf(leaf: post::Leaf, context: &ResolveContext) -> ResolveResult {
	let (column_definition, language) = leaf.field;
	let expression = Expr::col((
		table_alias(&context.alias, language),
		column_name(&column_definition),
	));

	let resolved_expression = match leaf.operation {
		post::Operation::Relation(relation) => todo!(),
		// TODO: need to handle escaping
		post::Operation::Match(string) => expression.like(format!("%{string}%")),
		post::Operation::Equal(value) => todo!(),
	};

	ResolveResult {
		condition: resolved_expression.into_condition(),
		languages: HashSet::from([language]),
	}
}

fn table_alias(alias_base: &str, language: Language) -> Alias {
	Alias::new(format!("{alias_base}@{}", LanguageString::from(language)))
}

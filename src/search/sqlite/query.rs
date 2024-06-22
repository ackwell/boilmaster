use std::collections::HashSet;

use ironworks::{
	excel::{Field, Language, Row, Sheet},
	file::exh,
};
use sea_query::{
	Alias, ColumnDef, ColumnRef, ColumnType, Condition, DynIden, Expr, Iden, InsertStatement,
	IntoColumnRef, IntoCondition, Order, Query, SelectStatement, SimpleExpr, Table,
	TableCreateStatement, TableDropStatement, TableRef, UnionType,
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

#[derive(Iden)]
enum KnownResolveColumn {
	Score,
}

pub fn resolve_queries(queries: Vec<(String, post::Node)>) -> SelectStatement {
	let selects = queries
		.into_iter()
		.map(|(sheet_name, node)| resolve_query(sheet_name, node));

	let mut query = selects
		.reduce(|mut a, b| a.union(UnionType::All, b).take())
		.expect("TODO: what if there's no queries?");

	query.order_by(KnownResolveColumn::Score, Order::Desc);
	// TODO: limit goes here

	query.take()
}

fn resolve_query(sheet_name: String, node: post::Node) -> SelectStatement {
	let alias = "alias-base";

	let ResolveResult {
		condition,
		score,
		languages,
		relations,
	} = resolve_node(
		node,
		&ResolveContext {
			alias,
			next_alias: "alias-0",
		},
	);

	let mut query = Query::select();

	// Base sheet and language joins.

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
	let (base_alias, base_reference) = table_references.next().expect("TODO: handle no languages");
	query.from(base_reference);
	for (join_alias, join_reference) in table_references {
		query.inner_join(
			join_reference,
			// TODO: this, and the copy below, only function on rowid - what about joining subrow sheets?
			Expr::col((join_alias, KnownColumn::RowId))
				.equals((base_alias.clone(), KnownColumn::RowId)),
		);
	}

	// Relations.
	for relation in relations {
		let mut relation_references = relation.languages.into_iter().map(|language| {
			let alias = table_alias(&relation.alias, language);
			(
				alias.clone(),
				TableRef::TableAlias(
					DynIden::new(table_name(&relation.sheet, language)),
					DynIden::new(alias),
				),
			)
		});

		let (base_alias, base_reference) = relation_references
			.next()
			.expect("TODO: handle no languages");
		query.left_join(
			base_reference,
			Expr::col(relation.foreign_key).equals((base_alias.clone(), KnownColumn::RowId)),
		);
		for (join_alias, join_reference) in relation_references {
			query.inner_join(
				join_reference,
				Expr::col((join_alias, KnownColumn::RowId))
					.equals((base_alias.clone(), KnownColumn::RowId)),
			);
		}
	}

	// Select fields.
	query.expr(Expr::val(&sheet_name));
	query.column((base_alias, KnownColumn::RowId));
	query.expr_as(score, KnownResolveColumn::Score);

	query.cond_where(condition);

	query.take()
}

struct ResolveContext<'a> {
	alias: &'a str,
	next_alias: &'a str,
}

struct ResolveResult {
	condition: Condition,
	score: SimpleExpr,
	languages: HashSet<Language>,
	relations: Vec<ResolveRelation>,
}

#[derive(Debug)]
struct ResolveRelation {
	sheet: String,
	alias: String,
	foreign_key: ColumnRef,
	languages: HashSet<Language>,
}

fn resolve_node(node: post::Node, context: &ResolveContext) -> ResolveResult {
	match node {
		post::Node::Group(group) => resolve_group(group, context),
		post::Node::Leaf(leaf) => resolve_leaf(leaf, context),
	}
}

fn resolve_group(group: post::Group, context: &ResolveContext) -> ResolveResult {
	let mut must = Condition::all();
	let mut should = Condition::any();
	let mut must_not = Condition::any().not();
	let mut score_expressions = vec![];
	let mut relations = vec![];

	let mut languages = HashSet::new();

	for (index, (occur, node)) in group.clauses.into_iter().enumerate() {
		let ResolveResult {
			condition: inner_condition,
			score: inner_score,
			languages: inner_languages,
			relations: inner_relations,
		} = resolve_node(
			node,
			&ResolveContext {
				alias: context.alias,
				next_alias: &format!("{}-{}", context.next_alias, index),
			},
		);

		match occur {
			// MUST: Score is gated by the entire group, add inner score directly.
			post::Occur::Must => {
				must = must.add(inner_condition);
				score_expressions.push(inner_score);
			}
			// SHOULD: Score needs to be gated per-expression.
			post::Occur::Should => {
				should = should.add(inner_condition.clone());
				score_expressions.push(Expr::case(inner_condition, inner_score).finally(0).into());
			}
			// MUSTNOT: Not scored.
			post::Occur::MustNot => must_not = must_not.add(inner_condition),
		}

		languages.extend(inner_languages);
		relations.extend(inner_relations);
	}

	// Add all the score expressions together.
	let mut score = score_expressions
		.into_iter()
		.reduce(|a, b| a.add(b))
		.unwrap_or_else(|| Expr::value(0));

	// If we have a MUST conditional, scope the scoring to require the MUSTs match first.
	if must.len() > 0 {
		score = Expr::case(must.clone(), score).finally(0).into();
	}

	// NOTE: we're only adding if c.len=0 here because any number of SHOULDs do not effect the _filtering_ of a query if there's 1 or more MUSTs - only the scoring. which i don't have any idea how to do. well, that's a lie. but still.
	if should.len() > 0 && must.len() == 0 {
		must = must.add(should)
	}

	if must_not.len() > 0 {
		must = must.add(must_not)
	}

	ResolveResult {
		condition: must,
		score,
		languages,
		relations,
	}
}

fn resolve_leaf(leaf: post::Leaf, context: &ResolveContext) -> ResolveResult {
	let mut relations = vec![];

	let (column_definition, language) = leaf.field;
	let column_ref = (
		table_alias(&context.alias, language),
		column_name(&column_definition),
	)
		.into_column_ref();
	let expression = Expr::col(column_ref.clone());

	let (resolved_expression, score) = match leaf.operation {
		// TODO: break this into seperate function?
		post::Operation::Relation(post::Relation { target, query }) => {
			let target_alias = context.next_alias.to_string();

			let ResolveResult {
				condition: inner_condition,
				score,
				languages,
				relations: inner_relations,
			} = resolve_node(
				*query,
				&ResolveContext {
					alias: &target_alias,
					next_alias: &format!("{}-0", target_alias),
				},
			);

			// TODO: Need to include target.condition (unscored) - possibly an Option<Condition> on the reference?
			let condition = expression
				.equals((table_alias(&target_alias, language), KnownColumn::RowId))
				.into_condition();

			relations.push(ResolveRelation {
				sheet: target.sheet,
				alias: target_alias,
				// condition,
				foreign_key: column_ref,
				languages,
			});
			relations.extend(inner_relations);

			(inner_condition, score)
		}

		// TODO: need to handle escaping
		// TODO: this is case insensitive due to LIKE semantics - if opting into case sensitive (is this something we want), will need to use GLOB or something with pragmas/collates, idk
		// TODO: score - can use stringlen / length(column)
		post::Operation::Match(string) => (
			expression.like(format!("%{string}%")).into_condition(),
			Expr::value(1),
		),

		post::Operation::Equal(value) => (expression.eq(value).into_condition(), Expr::value(1)),
	};

	ResolveResult {
		condition: resolved_expression.into_condition(),
		score,
		languages: HashSet::from([language]),
		relations,
	}
}

fn table_alias(alias_base: &str, language: Language) -> Alias {
	Alias::new(format!("{alias_base}@{}", LanguageString::from(language)))
}

impl From<post::Value> for sea_query::Value {
	fn from(value: post::Value) -> Self {
		match value {
			post::Value::U64(value) => sea_query::Value::BigUnsigned(Some(value)),
			post::Value::I64(value) => sea_query::Value::BigInt(Some(value)),
			post::Value::F64(value) => sea_query::Value::Double(Some(value)),
			post::Value::String(value) => sea_query::Value::String(Some(value.into())),
		}
	}
}

use std::{collections::HashSet, sync::OnceLock};

use aho_corasick::AhoCorasick;
use ironworks::excel::Language;
use sea_query::{
	Alias, ColumnRef, Condition, DynIden, Expr, Func, Iden, IntoColumnRef, IntoCondition, LikeExpr,
	Order, Query, SelectStatement, SimpleExpr, TableRef, UnionType,
};

use crate::{
	read::LanguageString,
	search::{
		error::{Error, Result},
		internal_query::post,
	},
};

use super::schema::{column_name, table_name, KnownColumn};

#[derive(Iden)]
enum KnownResolveColumn {
	Score,
}

pub fn resolve_queries(queries: Vec<(String, post::Node)>) -> Result<SelectStatement> {
	let mut selects = queries
		.into_iter()
		.map(|(sheet_name, node)| resolve_query(sheet_name, node));

	let mut query = selects
		.next()
		.ok_or_else(|| Error::MalformedQuery("no queries could be resolved".to_string()))??;
	for select in selects {
		query.union(UnionType::All, select?);
	}

	query.order_by(KnownResolveColumn::Score, Order::Desc);

	Ok(query.take())
}

fn resolve_query(sheet_name: String, node: post::Node) -> Result<SelectStatement> {
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
	)?;

	let mut query = Query::select();

	// Base sheet and language joins.
	let mut table_references = iter_language_references(languages, alias, &sheet_name);

	let (base_alias, base_reference) = table_references
		.next()
		.ok_or_else(|| Error::MalformedQuery("target sheet not referenced".to_string()))?;
	query.from(base_reference);

	inner_join_references(&mut query, table_references, &base_alias);

	// Relations.
	for relation in relations {
		let mut relation_references =
			iter_language_references(relation.languages, &relation.alias, &relation.sheet);

		// Use the first language to join the primary FK relation.
		let (base_alias, base_reference) = relation_references.next().ok_or_else(|| {
			Error::MalformedQuery(format!("joined sheet {} not referenced", relation.sheet))
		})?;

		let mut condition = Expr::col(relation.foreign_key)
			.equals((base_alias.clone(), KnownColumn::RowId))
			.into_condition();

		if let Some(relation_condition) = relation.condition {
			condition = Condition::all().add(condition).add(relation_condition);
		}

		query.left_join(base_reference, condition);

		// Remaining languages can be joined on the row ID.
		inner_join_references(&mut query, relation_references, &base_alias);
	}

	// Select fields.
	query.expr(Expr::val(&sheet_name));
	query.column((base_alias.clone(), KnownColumn::RowId));
	query.column((base_alias, KnownColumn::SubrowId));
	query.expr_as(score.cast_as(Alias::new("REAL")), KnownResolveColumn::Score);

	query.cond_where(condition);

	Ok(query.take())
}

fn iter_language_references<'a>(
	languages: impl IntoIterator<Item = Language> + 'a,
	alias: &'a str,
	sheet: &'a str,
) -> impl Iterator<Item = (Alias, TableRef)> + 'a {
	languages.into_iter().map(move |language| {
		let alias = table_alias(&alias, language);
		let reference = TableRef::TableAlias(
			DynIden::new(table_name(&sheet, language)),
			DynIden::new(alias.clone()),
		);
		(alias, reference)
	})
}

fn inner_join_references(
	query: &mut SelectStatement,
	references: impl Iterator<Item = (Alias, TableRef)>,
	target_alias: &Alias,
) {
	for (join_alias, join_reference) in references {
		query.inner_join(
			join_reference,
			Expr::col((join_alias, KnownColumn::RowId))
				.equals((target_alias.clone(), KnownColumn::RowId)),
		);
	}
}

struct ResolveContext<'a> {
	alias: &'a str,
	next_alias: &'a str,
}

#[derive(Debug)]
struct ResolveResult {
	/// Condition for the result tree.
	condition: Condition,
	/// Expression to calculate the score for the result tree.
	score: SimpleExpr,
	/// Languages used in the result tree that will need to be joined.
	languages: HashSet<Language>,
	/// Relationships required by this result tree.
	relations: Vec<ResolveRelation>,
}

#[derive(Debug)]
struct ResolveRelation {
	/// Sheet name.
	sheet: String,
	/// Alias used by this relationship.
	alias: String,
	/// Foreign key reference for the relationship join.
	foreign_key: ColumnRef,
	/// Additional constraints for the join.
	condition: Option<Condition>,
	/// Languages that will be required by the query for this relationship.
	languages: HashSet<Language>,
}

fn resolve_node(node: post::Node, context: &ResolveContext) -> Result<ResolveResult> {
	match node {
		post::Node::Group(group) => resolve_group(group, context),
		post::Node::Leaf(leaf) => resolve_leaf(leaf, context),
	}
}

fn resolve_group(group: post::Group, context: &ResolveContext) -> Result<ResolveResult> {
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
		)?;

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

	Ok(ResolveResult {
		condition: must,
		score,
		languages,
		relations,
	})
}

fn resolve_leaf(leaf: post::Leaf, context: &ResolveContext) -> Result<ResolveResult> {
	let mut relations = vec![];

	let (column_definition, language) = leaf.field;
	let column_ref = (
		table_alias(&context.alias, language),
		column_name(&column_definition),
	)
		.into_column_ref();
	let expression = Expr::col(column_ref.clone());
	let mut outer_languages = HashSet::from([language]);

	let (resolved_expression, score) = match leaf.operation {
		// TODO: break this into seperate function?
		post::Operation::Relation(post::Relation { target, query }) => {
			let target_alias = context.next_alias.to_string();

			let ResolveResult {
				condition: inner_condition,
				score,
				languages: inner_languages,
				relations: mut inner_relations,
			} = resolve_node(
				*query,
				&ResolveContext {
					alias: &target_alias,
					next_alias: &format!("{}-0", target_alias),
				},
			)?;

			let condition = match target.condition {
				None => None,
				Some(condition) => {
					// We don't care about score for these conditionals.
					let ResolveResult {
						condition: condition_condition,
						score: _,
						languages: condition_languages,
						relations: condition_relations,
					} = resolve_node(*condition, context)?;

					// NOTE: We need to merge the languages in with the outer set -
					// languages used in the condition are those of the current sheet.
					outer_languages.extend(condition_languages);
					inner_relations.extend(condition_relations);

					Some(condition_condition)
				}
			};

			relations.push(ResolveRelation {
				sheet: target.sheet,
				alias: target_alias,
				condition,
				foreign_key: column_ref,
				languages: inner_languages,
			});
			relations.extend(inner_relations);

			(inner_condition, score)
		}

		// TODO: this is case insensitive due to LIKE semantics - if opting into case sensitive (is this something we want), will need to use GLOB or something with pragmas/collates, idk
		post::Operation::Match(string) => (
			expression.like(build_like(&string)).into_condition(),
			Expr::value(u32::try_from(string.len()).map_err(|error| {
				Error::MalformedQuery(format!("excessively large string expression: {error}"))
			})?)
			.div(
				SimpleExpr::from(Func::char_length(Expr::col(column_ref)))
					.cast_as(Alias::new("REAL")),
			),
		),

		post::Operation::Eq(value) => (expression.eq(value).into_condition(), Expr::value(1)),

		post::Operation::Gt(number) => (expression.gt(number).into_condition(), Expr::value(1)),
		post::Operation::Gte(number) => (expression.gte(number).into_condition(), Expr::value(1)),
		post::Operation::Lt(number) => (expression.lt(number).into_condition(), Expr::value(1)),
		post::Operation::Lte(number) => (expression.lte(number).into_condition(), Expr::value(1)),
	};

	Ok(ResolveResult {
		condition: resolved_expression.into_condition(),
		score,
		languages: HashSet::from([language]),
		relations,
	})
}

fn build_like(string: &str) -> LikeExpr {
	static PATTERN: OnceLock<AhoCorasick> = OnceLock::new();
	let pattern = PATTERN.get_or_init(|| {
		AhoCorasick::new(["%", "_", "\\"]).expect("pattern construction should not fail")
	});

	let escaped = pattern.replace_all(string, &["\\%", "\\_", "\\\\"]);

	LikeExpr::new(format!("%{escaped}%")).escape('\\')
}

fn table_alias(alias_base: &str, language: Language) -> Alias {
	Alias::new(format!("{alias_base}@{}", LanguageString::from(language)))
}

impl From<post::Value> for sea_query::Value {
	fn from(value: post::Value) -> Self {
		match value {
			post::Value::Number(value) => sea_query::Value::from(value),
			post::Value::String(value) => sea_query::Value::String(Some(value.into())),
		}
	}
}

impl From<post::Number> for sea_query::Value {
	fn from(value: post::Number) -> Self {
		match value {
			post::Number::U64(value) => sea_query::Value::BigUnsigned(Some(value)),
			post::Number::I64(value) => sea_query::Value::BigInt(Some(value)),
			post::Number::F64(value) => sea_query::Value::Double(Some(value)),
		}
	}
}

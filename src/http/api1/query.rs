use std::str::FromStr;

use ironworks::excel;
use nom::{
	branch::alt,
	bytes::complete::{escaped_transform, is_not, tag},
	character::complete::{alphanumeric1, char, digit1, multispace1, one_of},
	combinator::{all_consuming, cut, map, map_res, not, opt, success, value as nom_value},
	multi::separated_list1,
	number::complete::double,
	sequence::{delimited, preceded, terminated, tuple},
	Finish, IResult,
};
use serde::{de, Deserialize};

use crate::{read, search::query};

use super::error;

// TODO: docs
#[derive(Debug)]
pub struct QueryString(query::Node);

impl From<QueryString> for query::Node {
	fn from(value: QueryString) -> Self {
		value.0
	}
}

impl<'de> Deserialize<'de> for QueryString {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

impl FromStr for QueryString {
	type Err = error::Error;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		// Root level of a query is an implicit group
		let (_rest, group) = all_consuming(group)(input)
			.finish()
			.map_err(|error| error::Error::Invalid(error.to_string()))?;

		Ok(Self(query::Node::Group(group)))
	}
}

type ParseResult<'a, T> = IResult<&'a str, T>;

fn node(input: &str) -> ParseResult<query::Node> {
	alt((
		map(delimited(char('('), group, char(')')), query::Node::Group),
		map(leaf, query::Node::Leaf),
	))(input)
}

fn group(input: &str) -> ParseResult<query::Group> {
	map(
		separated_list1(multispace1, tuple((occur, node))),
		|clauses| query::Group { clauses },
	)(input)
}

fn occur(input: &str) -> ParseResult<query::Occur> {
	alt((
		nom_value(query::Occur::Must, char('+')),
		nom_value(query::Occur::MustNot, char('-')),
		success(query::Occur::Should),
	))(input)
}

fn leaf(input: &str) -> ParseResult<query::Leaf> {
	map(
		tuple((struct_specifier, opt(array_specifier), operation)),
		|(struct_field, maybe_array_field, operation)| {
			let operation = match maybe_array_field {
				None => operation,
				Some(array_field) => operation_relation(query::Node::Leaf(query::Leaf {
					field: Some(array_field),
					operation: operation,
				})),
			};

			query::Leaf {
				field: Some(struct_field),
				operation,
			}
		},
	)(input)
}

fn struct_specifier(input: &str) -> ParseResult<query::FieldSpecifier> {
	map(
		tuple((
			alphanumeric1, // TODO: should this be an escaped transform?
			opt(preceded(char('@'), cut(language))),
		)),
		|(str, language)| query::FieldSpecifier::Struct(str.into(), language),
	)(input)
}

// TODO: this is duplicated with filter - share?
fn language(input: &str) -> ParseResult<excel::Language> {
	map_res(alphanumeric1, |str: &str| {
		str.parse::<read::LanguageString>()
			.map(excel::Language::from)
	})(input)
}

fn array_specifier(input: &str) -> ParseResult<query::FieldSpecifier> {
	nom_value(query::FieldSpecifier::Array, tag("[]"))(input)
}

fn operation(input: &str) -> ParseResult<query::Operation> {
	alt((
		preceded(char('.'), cut(map(node, operation_relation))),
		preceded(char('~'), cut(map(string, query::Operation::Match))),
		preceded(char('='), cut(map(value, query::Operation::Eq))),
		preceded(tag(">="), cut(map(number, query::Operation::Gte))),
		preceded(char('>'), cut(map(number, query::Operation::Gt))),
		preceded(tag("<="), cut(map(number, query::Operation::Lte))),
		preceded(char('<'), cut(map(number, query::Operation::Lt))),
	))(input)
}

fn value(input: &str) -> ParseResult<query::Value> {
	alt((
		map(number, query::Value::Number),
		map(string, query::Value::String),
	))(input)
}

fn number(input: &str) -> ParseResult<query::Number> {
	alt((
		// Try to parse the number as a potentially-signed integer. If it's followed by `.`, it'll fall through to the float check.
		terminated(
			alt((
				map(i64, query::Number::I64),
				map(map_res(digit1, str::parse), query::Number::U64),
			)),
			not(one_of(".eE")),
		),
		map(double, query::Number::F64),
	))(input)
}

fn i64(input: &str) -> ParseResult<i64> {
	map_res(preceded(char('-'), digit1), |value| -> anyhow::Result<_> {
		Ok(-i64::try_from(str::parse::<u64>(value)?)?)
	})(input)
}

fn string(input: &str) -> ParseResult<String> {
	delimited(
		char('"'),
		escaped_transform(
			is_not("\\\""),
			'\\',
			alt((
				//
				nom_value("\\", char('\\')),
				nom_value("\"", char('"')),
			)),
		),
		char('"'),
	)(input)
}

fn operation_relation(node: query::Node) -> query::Operation {
	query::Operation::Relation(query::Relation {
		target: (),
		query: Box::new(node),
	})
}

#[cfg(test)]
mod test {
	use pretty_assertions::assert_eq;

	use super::*;

	fn test_parse(input: &str) -> query::Node {
		let query_string = match input.parse::<QueryString>() {
			Ok(value) => value,
			Err(error) => {
				eprintln!("{error}");
				panic!();
			}
		};
		query_string.into()
	}

	fn group(clauses: Vec<(query::Occur, query::Node)>) -> query::Node {
		query::Node::Group(query::Group { clauses })
	}

	fn leaf(field: query::FieldSpecifier, operation: query::Operation) -> query::Node {
		query::Node::Leaf(query::Leaf {
			field: Some(field),
			operation,
		})
	}

	fn field_struct(key: impl ToString) -> query::FieldSpecifier {
		query::FieldSpecifier::Struct(key.to_string(), None)
	}

	fn operation_relation(node: query::Node) -> query::Operation {
		query::Operation::Relation(query::Relation {
			target: (),
			query: Box::new(node),
		})
	}

	fn u64(value: u64) -> query::Value {
		query::Value::Number(query::Number::U64(value))
	}

	#[test]
	fn parse_simple() {
		let expected = group(vec![(
			query::Occur::Should,
			leaf(field_struct("A"), query::Operation::Eq(u64(1))),
		)]);

		let got = test_parse("A=1");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_nested() {
		let expected = group(vec![(
			query::Occur::Should,
			leaf(
				field_struct("A"),
				operation_relation(leaf(field_struct("B"), query::Operation::Eq(u64(1)))),
			),
		)]);

		let got = test_parse("A.B=1");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_language() {
		let expected = group(vec![(
			query::Occur::Should,
			leaf(
				query::FieldSpecifier::Struct("A".into(), Some(excel::Language::Japanese)),
				query::Operation::Eq(u64(1)),
			),
		)]);

		let got = test_parse("A@ja=1");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_arrays() {
		let expected = group(vec![(
			query::Occur::Should,
			leaf(
				field_struct("A"),
				operation_relation(leaf(
					query::FieldSpecifier::Array,
					query::Operation::Eq(u64(1)),
				)),
			),
		)]);

		let got = test_parse("A[]=1");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_multiple() {
		let expected = group(vec![
			(
				query::Occur::Should,
				leaf(field_struct("A"), query::Operation::Eq(u64(1))),
			),
			(
				query::Occur::Should,
				leaf(field_struct("B"), query::Operation::Eq(u64(2))),
			),
		]);

		let got = test_parse("A=1 B=2");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_occur() {
		let expected = group(vec![
			(
				query::Occur::Should,
				leaf(field_struct("A"), query::Operation::Eq(u64(1))),
			),
			(
				query::Occur::Must,
				leaf(field_struct("B"), query::Operation::Eq(u64(2))),
			),
			(
				query::Occur::MustNot,
				leaf(field_struct("C"), query::Operation::Eq(u64(3))),
			),
		]);

		let got = test_parse("A=1 +B=2 -C=3");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_nested_groups() {
		let expected = group(vec![(
			query::Occur::Should,
			leaf(
				field_struct("A"),
				operation_relation(group(vec![
					(
						query::Occur::Should,
						leaf(field_struct("B"), query::Operation::Eq(u64(1))),
					),
					(
						query::Occur::Should,
						leaf(field_struct("C"), query::Operation::Eq(u64(2))),
					),
				])),
			),
		)]);

		let got = test_parse("A.(B=1 C=2)");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_operation() {
		fn harness(operation: query::Operation) -> query::Node {
			group(vec![(
				query::Occur::Should,
				leaf(field_struct("A"), operation),
			)])
		}

		assert_eq!(
			test_parse("A~\"hello\""),
			harness(query::Operation::Match("hello".into()))
		);

		assert_eq!(test_parse("A=1"), harness(query::Operation::Eq(u64(1))));

		assert_eq!(
			test_parse("A>=1"),
			harness(query::Operation::Gte(query::Number::U64(1)))
		);

		assert_eq!(
			test_parse("A>1"),
			harness(query::Operation::Gt(query::Number::U64(1)))
		);

		assert_eq!(
			test_parse("A<=1"),
			harness(query::Operation::Lte(query::Number::U64(1)))
		);

		assert_eq!(
			test_parse("A<1"),
			harness(query::Operation::Lt(query::Number::U64(1)))
		);
	}

	#[test]
	fn number_types() {
		fn harness(number: query::Number) -> query::Node {
			group(vec![(
				query::Occur::Should,
				leaf(
					field_struct("A"),
					query::Operation::Eq(query::Value::Number(number)),
				),
			)])
		}

		assert_eq!(test_parse("A=1"), harness(query::Number::U64(1)));
		assert_eq!(test_parse("A=-1"), harness(query::Number::I64(-1)));
		assert_eq!(test_parse("A=1.0"), harness(query::Number::F64(1.0)));
		assert_eq!(test_parse("A=1e0"), harness(query::Number::F64(1.0)));
		assert_eq!(test_parse("A=1E0"), harness(query::Number::F64(1.0)));
	}

	#[test]
	fn string_escaping() {
		fn harness(value: impl ToString) -> query::Node {
			group(vec![(
				query::Occur::Should,
				leaf(
					field_struct("A"),
					query::Operation::Match(value.to_string()),
				),
			)])
		}

		assert_eq!(test_parse(r#"A~"hello""#), harness(r#"hello"#));
		assert_eq!(test_parse(r#"A~"he'llo""#), harness(r#"he'llo"#));
		assert_eq!(test_parse(r#"A~"he\"llo""#), harness(r#"he"llo"#));
		assert_eq!(test_parse(r#"A~"he\\llo""#), harness(r#"he\llo"#));
	}
}

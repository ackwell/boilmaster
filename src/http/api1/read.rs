use ironworks::excel;
use nom::{
	branch::alt,
	bytes::complete::{tag, take_while1},
	character::complete::char,
	combinator::{map, map_res, opt, value},
	multi::separated_list1,
	sequence::{delimited, preceded, tuple},
	IResult,
};

use crate::{data, read2 as read};

struct FilterString(read::Filter);

// The root level of filters is effectively a struct, with optional braces. This
// is invalid anywhere but the root, as it makes commas ambiguous. We only allow
// it at the root to make trivial queries simple to write. This root also does
// not support arrays as the root filter node - this shouldn't be a problem, as
// all known sheet schemas have a struct with at least one field as root.
fn root_filter(input: &str) -> IResult<&str, read::Filter> {
	alt((
		alt((struct_, struct_fields)),
		value(read::Filter::All, opt(char('*'))),
	))(input)
}

fn filter(input: &str) -> IResult<&str, read::Filter> {
	preceded(
		opt(char('.')),
		alt((
			struct_,
			array,
			map(field, |field| read::Filter::Struct(vec![field])),
			value(read::Filter::All, opt(char('*'))),
		)),
	)(input)
}

fn struct_(input: &str) -> IResult<&str, read::Filter> {
	delimited(char('{'), struct_fields, char('}'))(input)
}

fn struct_fields(input: &str) -> IResult<&str, read::Filter> {
	map(separated_list1(char(','), field), |fields| {
		read::Filter::Struct(fields)
	})(input)
}

fn field(input: &str) -> IResult<&str, (read::StructKey, read::Filter)> {
	tuple((struct_key, filter))(input)
}

fn struct_key(input: &str) -> IResult<&str, read::StructKey> {
	map(tuple((alphanumeric, opt(language))), |(name, language)| {
		read::StructKey {
			name: name.into(),
			language,
		}
	})(input)
}

fn language(input: &str) -> IResult<&str, excel::Language> {
	map_res(preceded(char('@'), alphanumeric), |string| {
		string
			.parse::<data::LanguageString>()
			.map(excel::Language::from)
	})(input)
}

fn alphanumeric(input: &str) -> IResult<&str, &str> {
	// TODO: should i permit escaped tokens?
	take_while1(|c: char| c.is_ascii_alphanumeric())(input)
}

fn array(input: &str) -> IResult<&str, read::Filter> {
	map(
		// TODO: array indices
		tuple((tag("[]"), filter)),
		|(_, child)| read::Filter::Array(Box::new(child)),
	)(input)
}

#[cfg(test)]
mod test {
	use nom::Finish;

	use super::*;

	fn test_parse(input: &str) -> read::Filter {
		let (remaining, output) = root_filter(input).finish().expect("parse should not fail");
		// TODO: should i have a single entry point that uses all_consuming?
		assert_eq!(remaining, "");
		output
	}

	fn test_struct(
		entries: impl IntoIterator<Item = (impl ToString, read::Filter)>,
	) -> read::Filter {
		test_language_struct(entries.into_iter().map(|(key, value)| ((key, None), value)))
	}

	fn test_language_struct(
		entries: impl IntoIterator<Item = ((impl ToString, Option<excel::Language>), read::Filter)>,
	) -> read::Filter {
		read::Filter::Struct(
			entries
				.into_iter()
				.map(|((key, language), value)| {
					(
						read::StructKey {
							name: key.to_string(),
							language,
						},
						value,
					)
				})
				.collect(),
		)
	}

	fn test_array(child: read::Filter) -> read::Filter {
		read::Filter::Array(Box::new(child))
	}

	#[test]
	fn parse_all() {
		let expected = read::Filter::All;

		let got = test_parse("*");
		assert_eq!(got, expected);

		let got = test_parse("");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_simple() {
		let expected = test_struct([("a", read::Filter::All)]);

		let got = test_parse("{a.*}");
		assert_eq!(got, expected);

		let got = test_parse("a");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_language() {
		let expected =
			test_language_struct([(("a", Some(excel::Language::English)), read::Filter::All)]);

		let got = test_parse("{a@en.*}");
		assert_eq!(got, expected);

		let got = test_parse("a@en");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_nested() {
		let expected = test_struct([(
			"a",
			test_struct([("b", test_struct([("c", read::Filter::All)]))]),
		)]);

		let got = test_parse("{a.{b.{c.*}}}");
		assert_eq!(got, expected);

		let got = test_parse("a.b.c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_multiple_fields() {
		let expected = test_struct([(
			"a",
			test_struct([
				("b", test_struct([("c", read::Filter::All)])),
				("d", read::Filter::All),
			]),
		)]);

		let got = test_parse("{a.{b.{c.*},d.*}}");
		assert_eq!(got, expected);

		let got = test_parse("a.{b.c,d}");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_root_multiple_fields() {
		let expected = test_struct([
			("a", test_struct([("b", read::Filter::All)])),
			("c", read::Filter::All),
		]);

		let got = test_parse("{a.{b.*},c.*}");
		assert_eq!(got, expected);

		let got = test_parse("a.b,c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_root_shared_path() {
		let expected = test_struct([
			("a", test_struct([("b", read::Filter::All)])),
			("a", test_struct([("d", read::Filter::All)])),
		]);

		let got = test_parse("{a.{b.*},a.{c.*}}");
		assert_eq!(got, expected);

		let got = test_parse("a.b,a.c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_array_simple() {
		let expected = test_struct([("a", test_array(read::Filter::All))]);

		let got = test_parse("a.[].*");
		assert_eq!(got, expected);

		let got = test_parse("a[]");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_array_nested() {
		let expected = test_struct([(
			"a",
			test_array(test_array(test_struct([("b", read::Filter::All)]))),
		)]);

		let got = test_parse("{a.[].[].{b.*}}");
		assert_eq!(got, expected);

		let got = test_parse("a[][].b");
		assert_eq!(got, expected);
	}
}

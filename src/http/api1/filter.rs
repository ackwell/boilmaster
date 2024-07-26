use std::{collections::HashMap, str::FromStr};

use ironworks::excel;
use nohash_hasher::IntMap;
use nom::{
	branch::alt,
	bytes::complete::{escaped_transform, is_not, tag},
	character::complete::{alphanumeric1, char},
	combinator::{all_consuming, map, map_res, opt, value, verify},
	multi::{many0, separated_list0, separated_list1},
	sequence::{preceded, tuple},
	Finish, IResult,
};
use schemars::JsonSchema;
use serde::{de, Deserialize};

use crate::read;

use super::error;

/// A filter string for selecting fields within a row.
///
/// Filters are comprised of a comma-seperated list of field paths, i.e. `a,b`
/// will select the fields `a` and `b`.
///
/// A language may be specified on a field by field bases with an `@` suffix, i.e.
/// `a@ja` will select the field `a`, retrieving the Japanese data associated with it.
///
/// Nested fields may be selected using dot notation, i.e. `a.b` will select
/// the field `b` contained in the struct `a`.
///
/// Arrays must be targeted if selecting fields within them, i.e. `a[].b` will
/// select _all_ `b` fields of structs within the array `a`, however `a.b` will
/// select nothing.
#[derive(Debug, Clone, JsonSchema)]
pub struct FilterString(#[schemars(with = "String")] FilterStringInner);

#[derive(Debug, Clone)]
enum FilterStringInner {
	All,
	Paths(Vec<Path>),
}

type Path = Vec<Entry>;

#[derive(Debug, Clone)]
enum Entry {
	Key(String, Option<excel::Language>),
	Index,
}

impl FilterString {
	pub fn is_empty(&self) -> bool {
		match &self.0 {
			FilterStringInner::All => false,
			FilterStringInner::Paths(paths) => paths.is_empty(),
		}
	}

	pub fn to_filter(self, default_language: excel::Language) -> error::Result<read::Filter> {
		let paths = match self.0 {
			FilterStringInner::All => return Ok(read::Filter::All),
			FilterStringInner::Paths(paths) => paths,
		};

		let mut filters = paths
			.into_iter()
			.map(|entries| build_filter(entries, default_language));

		let Some(mut output) = filters.next() else {
			// TODO: Should I introduce an explicit "None" concept?
			return Ok(read::Filter::Struct(HashMap::new()));
		};

		for filter in filters {
			output = merge_filters(output, filter)?;
		}

		Ok(output)
	}
}

fn build_filter(path: Path, default_language: excel::Language) -> read::Filter {
	let mut output = read::Filter::All;

	// Walk through the path in reverse, building a nested filter structure for it
	for entry in path.into_iter().rev() {
		output = match entry {
			Entry::Index => read::Filter::Array(output.into()),

			Entry::Key(key, specified_language) => {
				let language = specified_language.unwrap_or(default_language);
				let mut language_map = IntMap::default();
				language_map.insert(read::Language(language), output);
				let key_map = HashMap::from([(key, language_map)]);
				read::Filter::Struct(key_map)
			}
		}
	}

	output
}

fn merge_filters(a: read::Filter, b: read::Filter) -> error::Result<read::Filter> {
	use read::Filter as F;

	let new_filter = match (a, b) {
		// If either branch is a catch-all, it propagates.
		(F::All, _) | (_, F::All) => F::All,

		// Arrays can directly merge their inner filter.
		(F::Array(a_inner), F::Array(b_inner)) => {
			F::Array(merge_filters(*a_inner, *b_inner)?.into())
		}

		// Structs need to be merged across both the inner maps.
		(F::Struct(mut a_fields), F::Struct(b_fields)) => {
			for (field_name, b_languages) in b_fields {
				let a_languages = a_fields.entry(field_name).or_default();
				for (language, b_filter) in b_languages {
					let new_filter = match a_languages.remove(&language) {
						None => b_filter,
						Some(a_filter) => merge_filters(a_filter, b_filter)?,
					};
					a_languages.insert(language, new_filter);
				}
			}
			F::Struct(a_fields)
		}

		// Other patterns are invalid. Explicitly checking the first element to
		// ensure this code path will error if new filter types are added.
		(F::Array(_), _) | (F::Struct(_), _) => {
			return Err(error::Error::Invalid(
				// TODO: improve this error message
				"invalid filter: tried to merge array and struct".into(),
			));
		}
	};

	Ok(new_filter)
}

impl<'de> Deserialize<'de> for FilterString {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let raw = String::deserialize(deserializer)?;
		raw.parse().map_err(de::Error::custom)
	}
}

impl FromStr for FilterString {
	// TODO: Is using the http error type "correct" here - it's the most relevant given _location_, but is it _relevant_?
	type Err = error::Error;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		// TODO: Consider using VerboseError or similar?
		let (_, filter) = all_consuming(filter)(input)
			.finish()
			.map_err(|error| error::Error::Invalid(error.to_string()))?;

		Ok(FilterString(filter))
	}
}

fn filter(input: &str) -> IResult<&str, FilterStringInner> {
	alt((
		value(FilterStringInner::All, char('*')),
		map(separated_list0(char(','), path), FilterStringInner::Paths),
	))(input)
}

fn path(input: &str) -> IResult<&str, Path> {
	map(separated_list1(char('.'), path_part), |parts| {
		parts.into_iter().flatten().collect()
	})(input)
}

fn path_part(input: &str) -> IResult<&str, Vec<Entry>> {
	map(tuple((key, many0(index))), |(key, mut maybe_index)| {
		let mut parts = vec![key];
		parts.append(&mut maybe_index);
		parts
	})(input)
}

fn key(input: &str) -> IResult<&str, Entry> {
	let escaped_key = escaped_transform(
		is_not("\\@[.,"),
		'\\',
		alt((
			value("\\", char('\\')),
			value("@", char('@')),
			value("[", char('[')),
			// NOTE: we don't actually need to support this, but it's nice QoL to permit balanced escapes.
			value("]", char(']')),
			value(".", char('.')),
			value(",", char(',')),
		)),
	);

	map(
		tuple((
			verify(escaped_key, |t: &str| !t.is_empty()),
			opt(preceded(char('@'), language)),
		)),
		|(key, language)| Entry::Key(key.into(), language),
	)(input)
}

fn index(input: &str) -> IResult<&str, Entry> {
	value(Entry::Index, tag("[]"))(input)
}

fn language(input: &str) -> IResult<&str, excel::Language> {
	map_res(alphanumeric1, |string: &str| {
		string
			.parse::<read::LanguageString>()
			.map(excel::Language::from)
	})(input)
}

#[cfg(test)]
mod test {
	use nohash_hasher::IntMap;
	use pretty_assertions::assert_eq;

	use super::*;

	fn test_parse(input: &str) -> read::Filter {
		let filter_string = input
			.parse::<FilterString>()
			.expect("parse should not fail");
		filter_string
			.to_filter(excel::Language::English)
			.expect("conversion should not fail")
	}

	fn test_struct(
		entries: impl IntoIterator<Item = (impl ToString, read::Filter)>,
	) -> read::Filter {
		test_language_struct(
			entries
				.into_iter()
				.map(|(key, value)| (key, test_language_map([(excel::Language::English, value)]))),
		)
	}

	fn test_language_struct(
		entries: impl IntoIterator<Item = (impl ToString, IntMap<read::Language, read::Filter>)>,
	) -> read::Filter {
		read::Filter::Struct(
			entries
				.into_iter()
				.map(|(key, languages)| (key.to_string(), languages))
				.collect(),
		)
	}

	fn test_language_map(
		entries: impl IntoIterator<Item = (excel::Language, read::Filter)>,
	) -> IntMap<read::Language, read::Filter> {
		entries
			.into_iter()
			.map(|(l, f)| (read::Language(l), f))
			.collect()
	}

	fn test_array(child: read::Filter) -> read::Filter {
		read::Filter::Array(Box::new(child))
	}

	#[test]
	fn parse_blank() {
		let expected = read::Filter::Struct(HashMap::new());

		let got = test_parse("");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_all() {
		let expected = read::Filter::All;

		let got = test_parse("*");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_simple() {
		let expected = test_struct([("a", read::Filter::All)]);

		let got = test_parse("a");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_language() {
		let expected = test_language_struct([(
			"a",
			test_language_map([(excel::Language::English, read::Filter::All)]),
		)]);

		let got = test_parse("a@en");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_nested() {
		let expected = test_struct([(
			"a",
			test_struct([("b", test_struct([("c", read::Filter::All)]))]),
		)]);

		let got = test_parse("a.b.c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_root_multiple_fields() {
		let expected = test_struct([
			("a", test_struct([("b", read::Filter::All)])),
			("c", read::Filter::All),
		]);

		let got = test_parse("a.b,c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_root_shared_path() {
		let expected = test_struct([(
			"a",
			test_struct([("b", read::Filter::All), ("c", read::Filter::All)]),
		)]);

		let got = test_parse("a.b,a.c");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_array_simple() {
		let expected = test_struct([("a", test_array(read::Filter::All))]);

		let got = test_parse("a[]");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_array_nested() {
		let expected = test_struct([(
			"a",
			test_array(test_array(test_struct([("b", read::Filter::All)]))),
		)]);

		let got = test_parse("a[][].b");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_complex_struct_keys() {
		let expected = test_struct([
			("curly{example}", read::Filter::All),
			("ltgt<example>", read::Filter::All),
			("parens(example)", read::Filter::All),
			("square[example]", read::Filter::All),
			("at@example", read::Filter::All),
			("comma,example", read::Filter::All),
			("period.example", read::Filter::All),
			("backslash\\example", read::Filter::All),
			("asterisk*example", read::Filter::All),
		]);

		let got = test_parse(
			"curly{example},ltgt<example>,parens(example),square\\[example\\],at\\@example,comma\\,example,period\\.example,backslash\\\\example,asterisk*example",
		);
		assert_eq!(got, expected);
	}
}

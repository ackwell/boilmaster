use std::{collections::HashMap, str::FromStr};

use ironworks::excel;
use nohash_hasher::IntMap;
use nom::{
	bytes::complete::{tag, take_while1},
	character::complete::char,
	combinator::{all_consuming, map, map_res, opt, value},
	multi::{many0, separated_list0, separated_list1},
	sequence::{preceded, tuple},
	Finish, IResult,
};
use serde::{de, Deserialize};

use crate::{data, read2 as read};

use super::error;

#[derive(Debug, Clone)]
pub struct FilterString(Vec<Path>);

type Path = Vec<Entry>;

#[derive(Debug, Clone)]
enum Entry {
	Key(String, Option<excel::Language>),
	Index,
}

impl FilterString {
	pub fn to_filter(self, default_language: excel::Language) -> error::Result<read::Filter> {
		let mut filters = self
			.0
			.into_iter()
			.map(|entries| build_filter(entries, default_language));

		let Some(mut output) = filters.next() else {
      return Ok(read::Filter::All);
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

		Ok(filter)
	}
}

fn filter(input: &str) -> IResult<&str, FilterString> {
	map(separated_list0(char(','), path), FilterString)(input)
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
	map(
		tuple((alphanumeric, opt(preceded(char('@'), language)))),
		|(key, language)| Entry::Key(key.into(), language),
	)(input)
}

fn index(input: &str) -> IResult<&str, Entry> {
	value(Entry::Index, tag("[]"))(input)
}

fn language(input: &str) -> IResult<&str, excel::Language> {
	map_res(alphanumeric, |string| {
		string
			.parse::<data::LanguageString>()
			.map(excel::Language::from)
	})(input)
}

fn alphanumeric(input: &str) -> IResult<&str, &str> {
	// TODO: should i permit escaped tokens?
	take_while1(|c: char| c.is_ascii_alphanumeric())(input)
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
	fn parse_all() {
		let expected = read::Filter::All;

		let got = test_parse("");
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
}

use std::{collections::HashMap, fmt, str::FromStr};

use bm_read as read;
use ironworks::excel;
use nom::{
	Finish, Parser,
	branch::alt,
	bytes::complete::{escaped_transform, is_not, tag},
	character::complete::{alphanumeric1, char},
	combinator::{all_consuming, consumed, cut, eof, map, map_res, value, verify},
	multi::{many0, separated_list0, separated_list1},
	sequence::{delimited, preceded},
};
use schemars::JsonSchema;
use serde::{Deserialize, de};

use super::error;

/// A filter string for selecting fields within a row.
///
/// Filters are comprised of a comma-seperated list of field paths, i.e. `a,b`
/// will select the fields `a` and `b`.
///
/// Decorators may be used to modify the way a field is read. They take the form
/// of `@decorator(arguments)`, i.e. `field@lang(en)`. Currently accepted
/// decorators:
///
/// - `@lang(<language>)`: Overrides the query's language for the decorated
///   field. Allows one query to access data for multiple languages. `language`
///   accepts any valid `LanguageString`.
///
/// - `@as(<format>)`: Overrides the default output format for the decorated
///   field.
///  
/// Currently accepted `format`s for `@as`:
///
/// - `raw`: Prevents further processing, such as sheet relations, being
///   performed on the decorated field. Has no effect on regular scalar fields.
///
/// - `html`: Formats a string field as rich HTML. Invalid on non-string
///   fields. Output will be a valid HTML fragment, however no stability
///   guarantees are made over the precise markup used.
///
/// Nested fields may be selected using dot notation, i.e. `a.b` will select the
/// field `b` contained in the struct `a`.
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
	Key {
		key: String,
		field: String,
		language: Option<excel::Language>,
		read_as: Option<read::As>,
	},
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

fn build_filter(mut path: Path, default_language: excel::Language) -> read::Filter {
	// If there's nothing in the path left, fall back to an all-selection.
	if path.is_empty() {
		return read::Filter::All;
	}

	let entry = path.drain(..1).next().expect("Ensured by check above");

	match entry {
		Entry::Index => read::Filter::Array(build_filter(path, default_language).into()),

		Entry::Key {
			key,
			field,
			language,
			read_as,
		} => {
			// Structs can override the default language of inner path entries.
			let inner_language = language.unwrap_or(default_language);

			read::Filter::Struct(HashMap::from([(
				key,
				read::StructEntry {
					field,
					language: inner_language,
					read_as: read_as.unwrap_or(read::As::Default),
					filter: build_filter(path, inner_language),
				},
			)]))
		}
	}
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

		// Structs need to have entry filters merged for matching keys.
		(F::Struct(mut a_fields), F::Struct(b_fields)) => {
			for (b_key, b_entry) in b_fields {
				let new_entry = match a_fields.remove(&b_key) {
					None => b_entry,

					// NOTE: This will technically kludge b's entry's non-filter
					// properties if there's a mismatch with a - however, given the
					// properties of entries are driven off the key in this filter
					// parser, there is no real opportunity for a mismatching entry for
					// a matching key.
					Some(a_entry) => read::StructEntry {
						filter: merge_filters(a_entry.filter, b_entry.filter)?,
						..a_entry
					},
				};
				a_fields.insert(b_key, new_entry);
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
		let (_, filter) = all_consuming(filter)
			.parse(input)
			.finish()
			.map_err(|error| error::Error::Invalid(error.to_string()))?;

		Ok(FilterString(filter))
	}
}

type IResult<I, O> = nom::IResult<I, O, ParseError<I>>;

#[derive(Debug)]
enum ParseError<I> {
	Nom(nom::error::Error<I>),
	Failure(String),
}

impl<I> nom::error::ParseError<I> for ParseError<I> {
	fn from_error_kind(input: I, kind: nom::error::ErrorKind) -> Self {
		Self::Nom(nom::error::Error::from_error_kind(input, kind))
	}

	fn append(_input: I, _kind: nom::error::ErrorKind, other: Self) -> Self {
		other
	}
}

impl<I, E> nom::error::FromExternalError<I, E> for ParseError<I> {
	fn from_external_error(input: I, kind: nom::error::ErrorKind, e: E) -> Self {
		Self::Nom(nom::error::Error::from_external_error(input, kind, e))
	}
}

impl<I> fmt::Display for ParseError<I>
where
	I: fmt::Display,
{
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Nom(inner) => inner.fmt(formatter),
			Self::Failure(message) => message.fmt(formatter),
		}
	}
}

fn filter(input: &str) -> IResult<&str, FilterStringInner> {
	alt((
		map(eof, |_| FilterStringInner::Paths(vec![])),
		value(FilterStringInner::All, char('*')),
		map(
			separated_list0(char(','), cut(path)),
			FilterStringInner::Paths,
		),
	))
	.parse(input)
}

fn path(input: &str) -> IResult<&str, Path> {
	map(separated_list1(char('.'), path_part), |parts| {
		parts.into_iter().flatten().collect()
	})
	.parse(input)
}

fn path_part(input: &str) -> IResult<&str, Vec<Entry>> {
	map((key, many0(index)), |(key, mut maybe_index)| {
		let mut parts = vec![key];
		parts.append(&mut maybe_index);
		parts
	})
	.parse(input)
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

	let (rest, (field, (decorator_input, decorators))) = (
		verify(escaped_key, |t: &str| !t.is_empty()),
		consumed(many0(decorator)),
	)
		.parse(input)?;

	let mut language = None;
	let mut read_as = None;

	(|| -> Result<(), &'static str> {
		for decorator in decorators {
			match decorator {
				Decorator::Language(d_lang) => set_option_once(&mut language, d_lang)?,
				Decorator::As(d_as) => set_option_once(&mut read_as, d_as)?,
			}
		}
		Ok(())
	})()
	.map_err(|message| {
		nom::Err::Failure(ParseError::Failure(format!("{message}: {decorator_input}")))
	})?;

	Ok((
		rest,
		Entry::Key {
			key: format!("{field}{decorator_input}"),
			field: field.into(),
			language,
			read_as,
		},
	))
}

fn set_option_once<T>(option: &mut Option<T>, value: T) -> Result<(), &'static str> {
	if option.is_some() {
		return Err("duplicate decorator");
	}

	*option = Some(value);

	Ok(())
}

fn index(input: &str) -> IResult<&str, Entry> {
	value(Entry::Index, tag("[]")).parse(input)
}

#[derive(Debug, Clone)]
enum Decorator {
	Language(excel::Language),
	As(read::As),
}

fn decorator(input: &str) -> IResult<&str, Decorator> {
	preceded(
		char('@'),
		alt((
			// Legacy support for un-prefixed languages
			map(language, Decorator::Language),
			// Call-syntax decorators
			map(call("lang", language), Decorator::Language),
			map(call("as", read_as), Decorator::As),
		)),
	)
	.parse(input)
}

fn call<'a, O, E, F>(name: &'a str, arguments: F) -> impl Parser<&'a str, Output = O, Error = E>
where
	E: nom::error::ParseError<&'a str>,
	F: Parser<&'a str, Output = O, Error = E>,
{
	preceded(tag(name), delimited(char('('), cut(arguments), char(')')))
}

fn language(input: &str) -> IResult<&str, excel::Language> {
	map_res(alphanumeric1, |string: &str| {
		string
			.parse::<read::LanguageString>()
			.map(excel::Language::from)
	})
	.parse(input)
}

fn read_as(input: &str) -> IResult<&str, read::As> {
	alt((
		value(read::As::Raw, tag("raw")),
		value(read::As::Html, tag("html")),
	))
	.parse(input)
}

#[cfg(test)]
mod test {
	use pretty_assertions::assert_eq;
	use read::StructEntry;

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
				.map(|(key, value)| (key.to_string(), key, excel::Language::English, value)),
		)
	}

	fn test_language_struct(
		entries: impl IntoIterator<Item = (impl ToString, impl ToString, excel::Language, read::Filter)>,
	) -> read::Filter {
		read::Filter::Struct(
			entries
				.into_iter()
				.map(|(key, field, language, filter)| {
					(
						key.to_string(),
						StructEntry {
							field: field.to_string(),
							language,
							read_as: read::As::Default,
							filter,
						},
					)
				})
				.collect(),
		)
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
	fn parse_struct_decorator_language() {
		let expected = test_language_struct([(
			"a@lang(en)",
			"a",
			excel::Language::English,
			read::Filter::All,
		)]);

		let got = test_parse("a@lang(en)");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_decorator_language_legacy() {
		let expected =
			test_language_struct([("a@en", "a", excel::Language::English, read::Filter::All)]);

		let got = test_parse("a@en");
		assert_eq!(got, expected);
	}

	#[test]
	fn parse_struct_decorator_duplicated() {
		let got = "a@lang(en)@lang(ja)".parse::<FilterString>();
		assert!(
			matches!(got, Err(error::Error::Invalid(message)) if message == "duplicate decorator: @lang(en)@lang(ja)")
		);
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

macro_rules! impl_jsonschema {
	($target:ident, $fn:ident) => {
		impl schemars::JsonSchema for $target {
			fn schema_name() -> ::std::string::String {
				::std::string::String::from(stringify!($target))
			}

			fn schema_id() -> ::std::borrow::Cow<'static, str> {
				::std::borrow::Cow::Borrowed(concat!(module_path!(), "::", stringify!($target)))
			}

			fn json_schema(
				generator: &mut ::schemars::r#gen::SchemaGenerator,
			) -> ::schemars::schema::Schema {
				$fn(generator)
			}
		}
	};
}

pub(crate) use impl_jsonschema;

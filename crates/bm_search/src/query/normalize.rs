use ironworks::{excel, file::exh};
use ironworks_schema as schema;

use crate::error::{Error, MismatchError, Result};

use super::{field, post, pre};

#[derive(Clone)]
struct Context<'a> {
	current_sheet: &'a str,
	languages: &'a [excel::Language],

	schema: &'a schema::Node,
	columns: &'a [exh::ColumnDefinition],
	language: excel::Language,

	ambient_language: excel::Language,

	path: &'a [&'a str],
}

impl Context<'_> {
	fn mismatch(&self, reason: impl ToString) -> MismatchError {
		MismatchError {
			field: self.path.join("."),
			reason: reason.to_string(),
		}
	}
}

pub struct Normalizer<'a> {
	excel: &'a excel::Excel,
	schema: &'a dyn schema::Schema,
}

impl<'a> Normalizer<'a> {
	pub fn new(excel: &'a excel::Excel, schema: &'a dyn schema::Schema) -> Self {
		Self { excel, schema }
	}

	#[inline]
	pub fn normalize(
		&self,
		query: &pre::Node,
		sheet_name: &str,
		ambient_language: excel::Language,
	) -> Result<post::Node> {
		self.normalize_query(query, sheet_name, ambient_language, &[])
	}

	fn normalize_query(
		&self,
		query: &pre::Node,
		sheet_name: &str,
		ambient_language: excel::Language,
		path: &[&str],
	) -> Result<post::Node> {
		// Fetch the schema and columns for the requested sheet.
		let sheet_schema = self.schema.sheet(sheet_name).map_err(|error| match error {
			// A missing schema can be considered analogous to a missing field _in_ a
			// schema, and is such a mismatch between the query and the schema.
			schema::Error::NotFound(inner) => Error::QuerySchemaMismatch(MismatchError {
				field: inner.to_string(),
				reason: "not found".into(),
			}),
			other => Error::Failure(other.into()),
		})?;

		let sheet_data = self.excel.sheet(sheet_name).map_err(|error| match error {
			ironworks::Error::NotFound(ironworks::ErrorValue::Sheet(sheet)) => {
				Error::SchemaGameMismatch(MismatchError {
					field: sheet,
					reason: "not found".into(),
				})
			}
			other => Error::Failure(other.into()),
		})?;

		let languages = sheet_data.languages()?;

		// TODO: this sort logic (along with much of the walking) is duplicated with read::
		//       work out if, and how, it could be shared.
		let mut columns = sheet_data.columns()?;
		match sheet_schema.order {
			schema::Order::Index => (),
			schema::Order::Offset => columns.sort_by_key(|column| column.offset()),
		}

		// Check if the ambient language is valid for this sheet, trying to fall
		// back to `None` if it is not, to mimic read behavior.
		let language = [ambient_language, excel::Language::None]
			.into_iter()
			.find(|language| languages.contains(language))
			.ok_or_else(|| {
				Error::QueryGameMismatch(MismatchError {
					field: format!("sheet {sheet_name}"),
					reason: format!("unsupported language {ambient_language:?}"),
				})
			})?;

		// Start walking the node tree
		self.normalize_node(
			query,
			Context {
				current_sheet: sheet_name,
				languages: &languages,
				schema: &sheet_schema.node,
				columns: &columns,
				language,
				ambient_language,
				path,
			},
		)
	}

	fn normalize_node(&self, node: &pre::Node, context: Context) -> Result<post::Node> {
		match node {
			pre::Node::Group(group) => self.normalize_group(group, context),
			pre::Node::Leaf(leaf) => self.normalize_leaf(leaf, context),
		}
	}

	fn normalize_group(&self, group: &pre::Group, context: Context) -> Result<post::Node> {
		Ok(post::Node::Group(post::Group {
			clauses: group
				.clauses
				.iter()
				.map(|(occur, node)| {
					Ok((occur.clone(), self.normalize_node(node, context.clone())?))
				})
				.collect::<Result<Vec<_>>>()?,
		}))
	}

	fn normalize_leaf(&self, leaf: &pre::Leaf, context: Context) -> Result<post::Node> {
		match &leaf.field {
			Some(specifier) => self.normalize_leaf_bound(specifier, &leaf.operation, context),
			None => self.normalize_leaf_unbound(&leaf.operation, context),
		}
	}

	fn normalize_leaf_bound(
		&self,
		specifier: &pre::FieldSpecifier,
		operation: &pre::Operation,
		context: Context,
	) -> Result<post::Node> {
		match (specifier, context.schema) {
			(
				pre::FieldSpecifier::Struct(field_name, requested_language),
				schema::Node::Struct(fields),
			) => self.normalize_leaf_bound_struct(
				operation,
				field_name,
				*requested_language,
				fields,
				context,
			),

			(pre::FieldSpecifier::Array(index), schema::Node::Array { count, node }) => {
				self.normalize_leaf_bound_array(operation, node, *count, *index, context)
			}

			// Anything other than a like-for-like match is, well, a mismatch.
			(specifier, node) => Err(Error::QuerySchemaMismatch(context.mismatch(format!(
				"cannot use {} query specifier for {} schema structures",
				match specifier {
					pre::FieldSpecifier::Struct(..) => "struct",
					pre::FieldSpecifier::Array(..) => "array",
				},
				match node {
					schema::Node::Array { .. } => "array",
					schema::Node::Scalar(..) => "scalar",
					schema::Node::Struct(..) => "struct",
				}
			)))),
		}
	}

	fn normalize_leaf_bound_struct(
		&self,
		operation: &pre::Operation,
		field_name: &str,
		requested_language: Option<excel::Language>,
		fields: &[schema::StructField],
		context: Context,
	) -> Result<post::Node> {
		// A struct specifier into a struct schema narrows the field space
		// TODO: should i encode the language at all? it's pretty irrelevant to most failures, but...
		let context = Context {
			path: &([context.path, &[field_name]].concat()),
			..context
		};

		// Get the requested field from the struct, mismatch if no such field
		// exists. Mismatch here implies the query and schema do not match.
		let field = fields
			.iter()
			// TODO: this is _really_ wasteful. see TODO in the utility file w/r/t sanitizing schema preemptively
			.find(|field| &field::sanitize_name(&field.name) == field_name)
			.ok_or_else(|| Error::QuerySchemaMismatch(context.mismatch("field does not exist")))?;

		// Get the requested language, falling back to the contextual language. We
		// do _not_ fall back to `Language::None` here - an explicit request for an
		// invalid language should fail. As-is, the contextual language is already
		// coerced to `Language::None` at the sheet boundary `.normalize` call, so
		// this will already fall back to `None` unless an erroneous language is
		// requested explicitly.
		let language = requested_language.unwrap_or(context.language);
		if !context.languages.contains(&language) {
			return Err(Error::QueryGameMismatch(
				context.mismatch(format!("{language:?} is not supported by this sheet")),
			));
		}

		// Narrow the column array to the columns relevant to the field, mismatch if
		// those columns do not exist. Mismatch here implies the game data and
		// schema do not match.
		let start = usize::try_from(field.offset).unwrap();
		let end = start + usize::try_from(field.node.size()).unwrap();
		let narrowed_columns = context.columns.get(start..end).ok_or_else(|| {
			Error::SchemaGameMismatch(context.mismatch("game data does not contain enough columns"))
		})?;

		// TODO: by leaving ambient_language as-is here, a query of `A@ja.B(Relation)` will fall back to the default language of the query for B.
		//       tempting to say that language overrides shouldn't spill outside their immediate field at all, honestly
		self.normalize_operation(
			operation,
			Context {
				schema: &field.node,
				columns: narrowed_columns,
				language,
				..context
			},
		)
	}

	fn normalize_leaf_bound_array(
		&self,
		operation: &pre::Operation,
		node: &schema::Node,
		count: u32,
		index: Option<u32>,
		context: Context,
	) -> Result<post::Node> {
		let path_entry = match index {
			None => std::borrow::Cow::Borrowed("[]"),
			Some(value) => std::borrow::Cow::Owned(format!("[{value}]")),
		};

		let context = Context {
			path: &([context.path, &[path_entry.as_ref()]].concat()),
			..context
		};

		let size = usize::try_from(node.size()).unwrap();

		// If there's an index, shortcut with a leaf node.
		if let Some(index) = index {
			let index_usize = usize::try_from(index).unwrap();
			return self.normalise_leaf_bound_array_index(
				operation,
				node,
				index_usize,
				size,
				context,
			);
		}

		let clauses = (0..usize::try_from(count).unwrap())
			.map(|index| -> Result<_> {
				let query = self.normalise_leaf_bound_array_index(
					operation,
					node,
					index,
					size,
					context.clone(),
				)?;

				Ok((post::Occur::Should, query))
			})
			.collect::<Result<Vec<_>>>()?;

		Ok(post::Node::Group(post::Group { clauses }))
	}

	fn normalise_leaf_bound_array_index(
		&self,
		operation: &pre::Operation,
		node: &schema::Node,
		index: usize,
		size: usize,
		context: Context,
	) -> Result<post::Node> {
		let start = index * size;
		let end = start + size;

		// TODO: This is duped, helper?
		let narrowed_columns = context.columns.get(start..end).ok_or_else(|| {
			Error::SchemaGameMismatch(context.mismatch("game data does not contain enough columns"))
		})?;

		self.normalize_operation(
			operation,
			Context {
				schema: node,
				columns: narrowed_columns,
				..context
			},
		)
	}

	fn normalize_leaf_unbound(
		&self,
		_operation: &pre::Operation,
		_context: Context,
	) -> Result<post::Node> {
		// TODO: notes; an unbound leaf only makes semantic sense on a structural schema node; were it pointing to a scalar node, it would be equivalent semantically to a bound leaf on that node. following from that; an unbound leaf should "fan out" to all of the current structural node's children as an or-group, in doing so effectively "consuming" the current node at the leaf point, which maintains consistency with bound leaf handling.

		Err(Error::MalformedQuery(
			"unbound query nodes are not currently supported".into(),
		))
	}

	fn normalize_operation(
		&self,
		operation: &pre::Operation,
		context: Context,
	) -> Result<post::Node> {
		match operation {
			pre::Operation::Relation(relation) => {
				self.normalize_operation_relation(relation, context)
			}

			pre::Operation::Match(string) => scalar_operation(
				|column| column.kind() == exh::ColumnKind::String,
				|| post::Operation::Match(string.clone()),
				context,
			),

			pre::Operation::Eq(value) => {
				scalar_operation(|_| true, || post::Operation::Eq(value.clone()), context)
			}

			pre::Operation::Gt(number) => scalar_operation(
				is_column_numeric,
				|| post::Operation::Gt(number.clone()),
				context,
			),
			pre::Operation::Gte(number) => scalar_operation(
				is_column_numeric,
				|| post::Operation::Gte(number.clone()),
				context,
			),
			pre::Operation::Lt(number) => scalar_operation(
				is_column_numeric,
				|| post::Operation::Lt(number.clone()),
				context,
			),
			pre::Operation::Lte(number) => scalar_operation(
				is_column_numeric,
				|| post::Operation::Lte(number.clone()),
				context,
			),
		}
	}

	fn normalize_operation_relation(
		&self,
		relation: &pre::Relation,
		context: Context,
	) -> Result<post::Node> {
		let targets = match context.schema {
			// Relations digging into schema structural features can be forwarded through to node normalisation.
			schema::Node::Struct(..) | schema::Node::Array { .. } => {
				return self.normalize_node(&relation.query, context)
			}

			schema::Node::Scalar(schema::Scalar::Reference(targets)) => targets,

			_other => Err(Error::QuerySchemaMismatch(
				context.mismatch("cannot perform relation operations on this schema node"),
			))?,
		};

		let field = match context.columns {
			[column] => column,
			other => {
				return Err(Error::SchemaGameMismatch(context.mismatch(format!(
					"cross-sheet references must have a single source (found {})",
					other.len()
				))))
			}
		};

		let target_queries = targets
			.iter()
			.map(|target| self.process_reference_target(field.clone(), target, relation, &context))
			// Filter out query mismatches to prune those branches - other errors will be raised.
			.filter(|result| !matches!(result, Err(Error::QuerySchemaMismatch(_))))
			.collect::<Result<Vec<_>, _>>()?;

		// There might be multiple viable relation paths, group them together.
		let node = create_or_group(target_queries.into_iter()).ok_or_else(|| {
			Error::QuerySchemaMismatch(
				context.mismatch("no target queries can be resolved against this schema"),
			)
		})?;

		Ok(node)
	}

	fn process_reference_target(
		&self,
		field: exh::ColumnDefinition,
		target: &schema::ReferenceTarget,
		relation: &pre::Relation,
		context: &Context,
	) -> Result<post::Node> {
		// TODO: This seems to be used for _one_ use case across all of stc, and is not used by EXDSchema at all. I don't think it's worth supporting, honestly.
		if target.selector.is_some() {
			return Err(Error::MalformedQuery(
				"search system does not currently support relationships with target selectors"
					.into(),
			));
		}

		// Normalise the relationship query.
		let query = self.normalize_query(
			&relation.query,
			&target.sheet,
			context.ambient_language,
			context.path, // TODO: Should this have an entry for the schema?
		)?;

		// If there's a condition on this relationship, also resolve that as a
		// seperate query - we're fabricating the pre:: query to create the
		// necessary selector mapping. Note that this is performed _after_ the
		// primary relationship query - this is to avoid needing to create these
		// conditions for relationship branches that fail out.
		let condition = match &target.condition {
			None => None,
			Some(condition) => {
				let condition_query = pre::Node::Leaf(pre::Leaf {
					// NOTE: This is letting the language fall through to ambient - I think that's correct?
					field: Some(pre::FieldSpecifier::Struct(
						condition.selector.clone(),
						None,
					)),
					operation: pre::Operation::Eq(pre::Value::Number(pre::Number::U64(
						condition.value.into(),
					))),
				});

				let node = self.normalize_query(
					&condition_query,
					context.current_sheet,
					context.ambient_language,
					context.path,
				)?;

				Some(Box::new(node))
			}
		};

		let operation = post::Operation::Relation(post::Relation {
			target: post::RelationTarget {
				sheet: target.sheet.clone(),
				condition,
			},
			query: Box::new(query),
		});

		let node = post::Node::Leaf(post::Leaf {
			field: (field.clone(), context.language),
			operation,
		});

		Ok(node)
	}
}

fn is_column_numeric(column: &exh::ColumnDefinition) -> bool {
	// NOTE: This is written to be comprehensive to ensure it does not drift if column kinds are updated.
	use exh::ColumnKind as CK;
	match column.kind() {
		CK::Int8
		| CK::UInt8
		| CK::Int16
		| CK::UInt16
		| CK::Int32
		| CK::UInt32
		| CK::Float32
		| CK::Int64
		| CK::UInt64 => true,

		CK::String
		| CK::Bool
		| CK::PackedBool0
		| CK::PackedBool1
		| CK::PackedBool2
		| CK::PackedBool3
		| CK::PackedBool4
		| CK::PackedBool5
		| CK::PackedBool6
		| CK::PackedBool7 => false,
	}
}

fn scalar_operation(
	filter: impl Fn(&exh::ColumnDefinition) -> bool,
	operation: impl Fn() -> post::Operation,
	context: Context,
) -> Result<post::Node> {
	let column = match context.columns {
		[column] => column,
		[] | [..] => {
			return Err(Error::QueryGameMismatch(
				context.mismatch("operations must target a single field"),
			))
		}
	};

	if !filter(column) {
		return Err(Error::QuerySchemaMismatch(context.mismatch(format!(
			"{:?} columns are invalid for this operation",
			column.kind()
		))));
	}

	Ok(post::Node::Leaf(post::Leaf {
		field: (column.clone(), context.language),
		operation: operation(),
	}))
}

fn create_or_group(mut nodes: impl Iterator<Item = post::Node>) -> Option<post::Node> {
	// Get first, if there is none, we can short circuit.
	let one = nodes.next()?;

	// If there was only one node, we can flatten by returning it directly.
	let two = match nodes.next() {
		None => return Some(one),
		Some(node) => node,
	};

	// Otherwise there's two or more nodes, create a group.
	let node = post::Node::Group(post::Group {
		clauses: [one, two]
			.into_iter()
			.chain(nodes)
			.map(|node| (post::Occur::Should, node))
			.collect(),
	});

	Some(node)
}

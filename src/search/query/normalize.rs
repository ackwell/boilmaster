use ironworks::{excel, file::exh};
use ironworks_schema as schema;

use crate::{
	search::error::{Error, MismatchError, Result},
	utility::field,
};

use super::{post, pre};

#[derive(Clone)]
struct Context<'a> {
	languages: &'a [excel::Language],

	schema: &'a schema::Node,
	columns: &'a [exh::ColumnDefinition],
	language: excel::Language,

	ambient_language: excel::Language,
}

pub struct Normalizer<'a> {
	excel: &'a excel::Excel,
	schema: &'a dyn schema::Schema,
}

impl<'a> Normalizer<'a> {
	pub fn new(excel: &'a excel::Excel, schema: &'a dyn schema::Schema) -> Self {
		Self { excel, schema }
	}

	pub fn normalize(
		&self,
		query: &pre::Node,
		sheet_name: &str,
		ambient_language: excel::Language,
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
				languages: &languages,
				schema: &sheet_schema.node,
				columns: &columns,
				language,
				ambient_language,
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
			// A struct specifier into a struct schema narrows the field space
			(
				pre::FieldSpecifier::Struct(field_name, requested_language),
				schema::Node::Struct(fields),
			) => {
				// Get the requested field from the struct, mismatch if no such field exists.
				// Mismatch here implies the query and schema do not match.
				let field = fields
					.iter()
					// TODO: this is _really_ wasteful. see TODO in the utility file w/r/t sanitizing schema preemptively
					.find(|field| &field::sanitize_name(&field.name) == field_name)
					.ok_or_else(|| {
						Error::QuerySchemaMismatch(MismatchError {
							field: field_name.into(),
							reason: "field does not exist".into(),
						})
					})?;

				// Get the requested language, falling back to the contextual language.
				// We do _not_ fall back to `Language::None` here - an explicit request
				// for an invalid language should fail. As-is, the contextual language
				// is already coerced to `Language::None` at the sheet boundary `.normalize`
				// call, so this will already fall back to `None` unless an erroneous
				// language is requested explicitly.
				let language = requested_language.unwrap_or(context.language);
				if !context.languages.contains(&language) {
					return Err(Error::QueryGameMismatch(MismatchError {
						field: field_name.into(),
						reason: format!("{language:?} is not supported by this sheet"),
					}));
				}

				// Narrow the column array to the columns relevant to the field, mismatch if those columns do not exist.
				// Mismatch here implies the game data and schema do not match.
				let start = usize::try_from(field.offset).unwrap();
				let end = start + usize::try_from(field.node.size()).unwrap();
				let narrowed_columns = context.columns.get(start..end).ok_or_else(|| {
					Error::SchemaGameMismatch(MismatchError {
						field: field_name.into(), // TODO: path
						reason: "game data does not contain enough columns".into(),
					})
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

			// TODO: reference
			// a (struct, reference) pair means... what
			// references are equivalent in data to a scalar, i.e. it's a leaf of an individual schema (though points to another)
			// i'm tempted to say that this should never occur. normalising the relation operation should handle references at that point, which would leave the inner leaf bound to already be pointing at something else. leaf bounds are inherently a structural detail, and scalars (and references) are not structural. think on that a bit more
			(pre::FieldSpecifier::Array, schema::Node::Array { count, node }) => {
				let size = usize::try_from(node.size()).unwrap();
				let clauses = (0..usize::try_from(*count).unwrap())
					.map(|index| -> Result<_> {
						let start = index * size;
						let end = start + size;

						// TODO: This is duped, helper?
						let narrowed_columns =
							context.columns.get(start..end).ok_or_else(|| {
								Error::SchemaGameMismatch(MismatchError {
									field: "TODO: query path".into(),
									reason: "game data does not contain enough columns".into(),
								})
							})?;

						let query = self.normalize_operation(
							operation,
							Context {
								schema: node,
								columns: narrowed_columns,
								..context
							},
						)?;

						Ok((post::Occur::Should, query))
					})
					.collect::<Result<Vec<_>>>()?;

				Ok(post::Node::Group(post::Group { clauses }))
			}

			// Anything other than a like-for-like match is, well, a mismatch.
			(specifier, node) => Err(Error::QuerySchemaMismatch(MismatchError {
				field: "TODO query".into(),
				reason: format!(
					"cannot use {} query specifier for {} schema structures",
					match specifier {
						pre::FieldSpecifier::Struct(..) => "struct",
						pre::FieldSpecifier::Array => "array",
					},
					match node {
						schema::Node::Array { .. } => "array",
						schema::Node::Scalar(..) => "scalar",
						schema::Node::Struct(..) => "struct",
					}
				),
			})),
		}
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
			// TODO: should this panic if it _isn't_ a 1:1 relation:reference pair?
			//       no, it shouldn't - it could also be a struct... wait, can it?
			//       yeah, the callsite might have drilled into a struct, but this relation forms the basis of the next target, i think
			// so tldr;
			// for relations, if the schema is a reference, resolve the reference. if it's a struct, call down. if it's anything else, throw?
			pre::Operation::Relation(relation) => {
				let node = match context.schema {
					// Relations digging into schema structural features can be forwarded through to node normalisation.
					schema::Node::Struct(..) | schema::Node::Array { .. } => {
						self.normalize_node(&relation.query, context)?
					}

					schema::Node::Scalar(schema::Scalar::Reference(targets)) => {
						let field = match context.columns {
							[column] => column,
							other => {
								return Err(Error::SchemaGameMismatch(MismatchError {
									field: "TODO: query path".into(),
									reason: format!(
										"cross-sheet references must have a single source (found {})",
										other.len()
									),
								}))
							}
						};

						let target_queries = targets
							.iter()
							.map(|target| {
								// this seems to be used for _one_ use case across all of stc - look into if it's worth supporting
								if target.selector.is_some() {
									todo!("todo: normalise reference target selectors")
								}

								// this should be modelled as a boolean group (+condition +innerquery)
								if target.condition.is_some() {
									todo!("TODO: normalise reference target conditions")
								}

								let query = self.normalize(
									&relation.query,
									&target.sheet,
									context.ambient_language,
								)?;

								let operation = post::Operation::Relation(post::Relation {
									target: post::RelationTarget {
										sheet: target.sheet.clone(),
										condition: None, // todo
									},
									query: Box::new(query),
								});

								let node = post::Node::Leaf(post::Leaf {
									field: (field.clone(), context.language),
									operation,
								});

								Ok(node)
							})
							// Filter out query mismatches to prune those branches - other errors will be raised.
							.filter(|result| !matches!(result, Err(Error::QuerySchemaMismatch(_))))
							.collect::<Result<Vec<_>, _>>()?;

						// TODO: target_queries.len() == 0 here means none of the relations matched, which should be raised as a query mismatch

						// There might be multiple viable relation paths, group them together.
						create_or_group(target_queries.into_iter()).ok_or_else(|| {
							Error::QuerySchemaMismatch(MismatchError {
								field: "TODO: query path".into(),
								reason: "no target queries can be resolved against this schema"
									.into(),
							})
						})?
					}

					_other => Err(Error::QuerySchemaMismatch(MismatchError {
						field: "TODO: query path".into(),
						reason: "cannot perform relation operations on this schema node".into(),
					}))?,
				};

				Ok(node)
			}

			pre::Operation::Match(string) => {
				let scalar_columns = collect_scalars(context.schema, context.columns, vec![])
					.ok_or_else(|| {
						Error::SchemaGameMismatch(MismatchError {
							// TODO: i'll need to wire down the current query path for this field to be meaningful
							field: "TODO: query path".into(),
							reason: "insufficient game data to satisfy schema".into(),
						})
					})?;

				let string_columns = scalar_columns
					.into_iter()
					.filter(|column| column.kind() == exh::ColumnKind::String)
					.collect::<Vec<_>>();

				let group = create_or_group(string_columns.into_iter().map(|column| {
					post::Node::Leaf(post::Leaf {
						field: (column, context.language),
						operation: post::Operation::Match(string.clone()),
					})
				}))
				.ok_or_else(|| {
					Error::QuerySchemaMismatch(MismatchError {
						// TODO: i'll need to wire down the current query path for this field to be meaningful
						field: "TODO: query path".into(),
						reason: "no string columns with this name exist.".into(),
					})
				})?;

				Ok(group)
			}

			// TODO: this should collect all scalars i think?
			// TODO: this pattern will be pretty repetetive, make a utility that does this or something
			pre::Operation::Equal(value) => {
				let scalar_columns = collect_scalars(context.schema, context.columns, vec![])
					.ok_or_else(|| {
						Error::SchemaGameMismatch(MismatchError {
							// TODO: i'll need to wire down the current query path for this field to be meaningful
							field: "TODO: query path".into(),
							reason: "insufficient game data to satisfy schema".into(),
						})
					})?;

				let group = create_or_group(scalar_columns.into_iter().map(|column| {
					post::Node::Leaf(post::Leaf {
						field: (column, context.language),
						operation: post::Operation::Equal(value.clone()),
					})
				}))
				.ok_or_else(|| {
					Error::QueryGameMismatch(MismatchError {
						// TODO: i'll need to wire down the current query path for this field to be meaningful
						field: "TODO: query path".into(),
						reason: "no scalar columns with this name exist".into(),
					})
				})?;

				Ok(group)
			}
		}
	}
}

fn create_or_group(mut nodes: impl ExactSizeIterator<Item = post::Node>) -> Option<post::Node> {
	let node = match nodes.len() {
		0 => return None,
		1 => nodes.next().unwrap(),
		_ => post::Node::Group(post::Group {
			clauses: nodes.map(|node| (post::Occur::Should, node)).collect(),
		}),
	};

	Some(node)
}

// The whole premise of this is that we want to _exclude_ references. If that premise does not hold, then the `columns` slice itself is basically exactly what we want.
// TODO: On discussing, people(singular) seemed okay with a field being simultaneously scalar and a reference. I can't say I'm convinced, but it might be fine.
fn collect_scalars(
	schema: &schema::Node,
	columns: &[exh::ColumnDefinition],
	mut output: Vec<exh::ColumnDefinition>,
) -> Option<Vec<exh::ColumnDefinition>> {
	match schema {
		schema::Node::Array { count, node } => {
			// TODO: this is pretty silly, can technically derive the range from 1 call down.
			let size = usize::try_from(node.size()).unwrap();
			let count = usize::try_from(*count).unwrap();
			(0..count).try_fold(output, |output, index| {
				let start = index * size;
				let end = start + size;
				let slice = columns.get(start..end)?;
				collect_scalars(node, slice, output)
			})
		}

		schema::Node::Scalar(scalar) => {
			match scalar {
				schema::Scalar::Reference(_references) => {
					// ignore references
				}

				_other => {
					output.push(columns.get(0)?.clone());
				}
			}
			Some(output)
		}

		schema::Node::Struct(fields) => fields.iter().try_fold(output, |output, field| {
			let start = usize::try_from(field.offset).unwrap();
			let end = start + usize::try_from(field.node.size()).unwrap();
			let slice = columns.get(start..end)?;
			collect_scalars(&field.node, slice, output)
		}),
	}
}

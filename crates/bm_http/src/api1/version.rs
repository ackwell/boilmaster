use aide::{
	axum::{ApiRouter, routing::get_with},
	transform::TransformOperation,
};
use axum::{Json, debug_handler, extract::State};
use schemars::JsonSchema;
use serde::Serialize;

use crate::service::Service;

use super::api::ApiState;

pub fn router(state: ApiState) -> ApiRouter {
	ApiRouter::new().api_route("/", get_with(versions, versions_docs).with_state(state))
}

/// Response structure for the version endpoint.
#[derive(Serialize, JsonSchema)]
struct VersionsResponse {
	/// Array of versions available in the API.
	versions: Vec<VersionMetadata>,
}

/// Metadata about a single version supported by the API.
#[derive(Serialize, JsonSchema)]
struct VersionMetadata {
	/// Names associated with this version. Version names specified here are
	/// accepted by the `version` query parameter throughout the API.
	names: Vec<String>,

	// Used for sorting.
	#[serde(skip)]
	patch: String,
}

fn versions_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list versions")
		.description("List versions understood by the API.")
		.response_with::<200, Json<VersionsResponse>, _>(|response| {
			response.example(VersionsResponse {
				versions: vec![
					VersionMetadata {
						names: vec!["7.01".into(), "latest".into()],
						patch: "unused".into(),
					},
					VersionMetadata {
						names: vec!["7.0".into()],
						patch: "unused".into(),
					},
				],
			})
		})
}

#[debug_handler(state = ApiState)]
async fn versions(State(Service { version, .. }): State<Service>) -> Json<VersionsResponse> {
	let mut metadata = version
		.keys()
		.into_iter()
		// Given the list of keys is from the version manager, we should never hit a
		// None here - but be safe just in case.
		.filter_map(|key| {
			let names = version.names(key)?;
			let data = version.version(key)?;
			let patch = data.repositories.first()?.latest().name.clone();
			Some(VersionMetadata { names, patch })
		})
		.collect::<Vec<_>>();

	metadata.sort_unstable_by(|a, b| a.patch.cmp(&b.patch));

	Json(VersionsResponse { versions: metadata })
}

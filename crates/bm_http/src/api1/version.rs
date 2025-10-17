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
					},
					VersionMetadata {
						names: vec!["7.0".into()],
					},
				],
			})
		})
}

#[debug_handler(state = ApiState)]
async fn versions(State(Service { version, .. }): State<Service>) -> Json<VersionsResponse> {
	let version_keys = version.keys();

	let metadata = version_keys
		.into_iter()
		// Given the list of keys is from the version manager, we should never hit a
		// None here - but be safe just in case.
		.filter_map(|key| version.names(key).map(|names| VersionMetadata { names }))
		.collect();

	Json(VersionsResponse { versions: metadata })
}

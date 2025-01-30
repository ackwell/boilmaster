use aide::{
	axum::{routing::get_with, ApiRouter},
	transform::TransformOperation,
};
use axum::{debug_handler, extract::State, Json};
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
						names: vec!["latest".into()],
					},
					VersionMetadata {
						names: vec!["7.0".into()],
					},
					VersionMetadata {
						names: vec!["7.01".into()],
					},
				],
			})
		})
}

#[debug_handler(state = ApiState)]
async fn versions(State(Service { version, .. }): State<Service>) -> Json<VersionsResponse> {
	let mut names = version.all_names();
	names.sort_unstable();

	let metadata = names
		.into_iter()
		.map(|name| VersionMetadata { names: vec![name] })
		.collect();

	Json(VersionsResponse { versions: metadata })
}

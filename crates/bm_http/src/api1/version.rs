use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	transform::TransformOperation,
};
use axum::{debug_handler, extract::State, Json};

use crate::service::Service;

use super::api::ApiState;

pub fn router(state: ApiState) -> ApiRouter {
	ApiRouter::new().api_route("/", get_with(versions, versions_docs).with_state(state))
}

fn versions_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list versions")
		.description("List valid version names accepted by the `version` query parameter.")
		.response_with::<200, Json<Vec<&'static str>>, _>(|response| {
			response.example(vec!["latest", "7.0", "7.01"])
		})
}

#[debug_handler(state = ApiState)]
async fn versions(State(Service { version, .. }): State<Service>) -> impl IntoApiResponse {
	let mut names = version.all_names();
	names.sort_unstable();
	Json(names)
}

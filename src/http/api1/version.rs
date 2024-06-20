use aide::{
	axum::{routing::get_with, ApiRouter, IntoApiResponse},
	transform::TransformOperation,
};
use axum::{debug_handler, extract::State, Json};

use crate::http::service;

pub fn router() -> ApiRouter<service::State> {
	ApiRouter::new().api_route("/", get_with(versions, versions_docs))
}

fn versions_docs(operation: TransformOperation) -> TransformOperation {
	operation
		.summary("list versions")
		.description("List valid version names accepted by the `version` query parameter.")
		.response_with::<200, Json<Vec<&'static str>>, _>(|response| {
			response.example(vec!["latest", "6.58", "6.58x1"])
		})
}

#[debug_handler(state = service::State)]
async fn versions(State(version): State<service::Version>) -> impl IntoApiResponse {
	let mut names = version.all_names();
	names.sort_unstable();
	Json(names)
}

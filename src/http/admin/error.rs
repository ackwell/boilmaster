use axum::response::{IntoResponse, Response};
use reqwest::StatusCode;

#[derive(Debug)]
pub struct Error(anyhow::Error);

impl<E> From<E> for Error
where
	E: Into<anyhow::Error>,
{
	fn from(value: E) -> Self {
		Self(value.into())
	}
}

impl IntoResponse for Error {
	fn into_response(self) -> Response {
		(
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("error: {}", self.0),
		)
			.into_response()
	}
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

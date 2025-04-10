use aide::{OperationOutput, openapi::Response as AideResponse, transform::TransformResponse};
use axum::{
	Json,
	extract::rejection::{PathRejection, QueryRejection},
	http::StatusCode,
	response::{IntoResponse, Response as AxumResponse},
};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("not found: {0}")]
	NotFound(String),

	#[error("invalid request: {0}")]
	Invalid(String),

	#[error("unavailable: {0}")]
	Unavailable(String),

	#[error("internal server error")]
	Other(#[from] anyhow::Error),
}

impl From<bm_asset::Error> for Error {
	fn from(error: bm_asset::Error) -> Self {
		use bm_asset::Error as AE;
		match error {
			AE::NotFound(..) => Self::NotFound(error.to_string()),
			AE::UnsupportedSource(..) | AE::InvalidConversion(..) | AE::UnknownFormat(..) => {
				Self::Invalid(error.to_string())
			}
			AE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<bm_data::Error> for Error {
	fn from(error: bm_data::Error) -> Self {
		use bm_data::Error as DE;
		match error {
			DE::UnknownVersion(..) => Self::Invalid(error.to_string()),
			DE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<bm_read::Error> for Error {
	fn from(error: bm_read::Error) -> Self {
		use bm_read::Error as RE;
		match error {
			RE::NotFound(..) => Self::NotFound(error.to_string()),
			RE::FilterSchemaMismatch(..) | RE::SchemaGameMismatch(..) | RE::InvalidLanguage(..) => {
				Self::Invalid(error.to_string())
			}
			RE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<bm_schema::Error> for Error {
	fn from(error: bm_schema::Error) -> Self {
		use bm_schema::Error as SE;
		match error {
			SE::UnknownSource(..) | SE::InvalidVersion(..) => Self::Invalid(error.to_string()),
			SE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<bm_search::Error> for Error {
	fn from(error: bm_search::Error) -> Self {
		use bm_search::Error as SE;
		match error {
			SE::NotReady => Self::Unavailable(error.to_string()),
			SE::FieldType(..)
			| SE::MalformedQuery(..)
			| SE::QuerySchemaMismatch(..)
			| SE::QueryGameMismatch(..)
			| SE::SchemaGameMismatch(..)
			| SE::UnknownCursor(..) => Self::Invalid(error.to_string()),
			SE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<PathRejection> for Error {
	fn from(value: PathRejection) -> Self {
		match value {
			PathRejection::FailedToDeserializePathParams(error) => Self::Invalid(error.body_text()),
			other => Self::Other(other.into()),
		}
	}
}

impl From<QueryRejection> for Error {
	fn from(value: QueryRejection) -> Self {
		match value {
			QueryRejection::FailedToDeserializeQueryString(error) => {
				Self::Invalid(error.body_text())
			}
			other => Self::Other(other.into()),
		}
	}
}

macro_rules! impl_to_failure {
	($source:ty) => {
		impl From<$source> for Error {
			fn from(value: $source) -> Self {
				Self::Other(value.into())
			}
		}
	};
}

impl_to_failure!(ironworks::Error);

/// General purpose error response structure.
#[derive(Serialize, JsonSchema)]
pub struct ErrorResponse {
	/// HTTP status code of the error. Will match the server response code.
	#[serde(with = "StatusCodeDef")]
	code: StatusCode,

	/// Description of what went wrong.
	message: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(remote = "StatusCode")]
struct StatusCodeDef(#[serde(getter = "StatusCode::as_u16")] u16);

impl From<Error> for ErrorResponse {
	fn from(value: Error) -> Self {
		// TODO: INCREDIBLY IMPORTANT: work out how to worm IM_A_TEAPOT into this
		let status_code = match value {
			Error::NotFound(..) => StatusCode::NOT_FOUND,
			Error::Invalid(..) => StatusCode::BAD_REQUEST,
			Error::Unavailable(..) => StatusCode::SERVICE_UNAVAILABLE,
			Error::Other(..) => StatusCode::INTERNAL_SERVER_ERROR,
		};

		Self {
			code: status_code,
			message: value.to_string(),
		}
	}
}

impl IntoResponse for Error {
	fn into_response(self) -> AxumResponse {
		// Log the full error for ISEs - we don't show this info anywhere else in case it contains something sensitive.
		if let Self::Other(ref error) = self {
			tracing::error!("{error:?}")
		}

		let response = ErrorResponse::from(self);

		(response.code, Json(response)).into_response()
	}
}

impl OperationOutput for Error {
	type Inner = ErrorResponse;

	fn inferred_responses(
		ctx: &mut aide::generate::GenContext,
		operation: &mut aide::openapi::Operation,
	) -> Vec<(Option<u16>, AideResponse)> {
		let Some(mut error_response) = Json::<ErrorResponse>::operation_response(ctx, operation)
		else {
			return vec![];
		};

		let _ = TransformResponse::<ErrorResponse>::new(&mut error_response)
			.description("failed operation")
			.example(Error::Invalid("example error response".into()));

		// NOTE: Using `None` here as otherwise we bloat out responses with a bunch of copy paste errors. Is there a better approach?
		vec![(None, error_response)]
	}
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

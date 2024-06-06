use axum::{
	extract::rejection::{PathRejection, QueryRejection},
	http::StatusCode,
	response::{IntoResponse, Response},
	Json,
};
use serde::Serialize;

use crate::{asset, data, read, schema, search};

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("not found: {0}")]
	NotFound(String),

	#[error("invalid request: {0}")]
	Invalid(String),

	// #[error("unavailable: {0}")]
	// Unavailable(String),
	//
	#[error("internal server error")]
	Other(#[from] anyhow::Error),
}

impl From<asset::Error> for Error {
	fn from(error: asset::Error) -> Self {
		use asset::Error as AE;
		match error {
			AE::NotFound(value) => Self::NotFound(value),
			AE::UnsupportedSource(..) | AE::InvalidConversion(..) | AE::UnknownFormat(..) => {
				Self::Invalid(error.to_string())
			}
			AE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<data::Error> for Error {
	fn from(error: data::Error) -> Self {
		use data::Error as DE;
		match error {
			DE::UnknownVersion(..) | DE::UnknownLanguage(..) => Self::Invalid(error.to_string()),
			DE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<read::Error> for Error {
	fn from(error: read::Error) -> Self {
		use read::Error as RE;
		match error {
			RE::FilterSchemaMismatch(..) | RE::SchemaGameMismatch(..) => {
				Self::Invalid(error.to_string())
			}
			RE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<schema::Error> for Error {
	fn from(error: schema::Error) -> Self {
		use schema::Error as SE;
		match error {
			SE::UnknownSource(..) | SE::InvalidVersion(..) => Self::Invalid(error.to_string()),
			SE::Failure(inner) => Self::Other(inner),
		}
	}
}

impl From<search::Error> for Error {
	fn from(error: search::Error) -> Self {
		use search::Error as SE;
		match error {
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

#[derive(Serialize)]
struct ErrorResponse {
	code: u16,
	message: String,
}

impl IntoResponse for Error {
	fn into_response(self) -> Response {
		// Log the full error for ISEs - we don't show this info anywhere else in case it contains something sensitive.
		if let Self::Other(ref error) = self {
			tracing::error!("{error:?}")
		}

		// TODO: INCREDIBLY IMPORTANT: work out how to worm IM_A_TEAPOT into this
		let status_code = match self {
			Self::NotFound(..) => StatusCode::NOT_FOUND,
			Self::Invalid(..) => StatusCode::BAD_REQUEST,
			// Self::Unavailable(..) => StatusCode::SERVICE_UNAVAILABLE,
			Self::Other(..) => StatusCode::INTERNAL_SERVER_ERROR,
		};

		(
			status_code,
			Json(ErrorResponse {
				code: status_code.as_u16(),
				message: self.to_string(),
			}),
		)
			.into_response()
	}
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

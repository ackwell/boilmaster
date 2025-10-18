use aide::OperationIo;
use axum::{
	RequestPartsExt,
	extract::{FromRef, FromRequestParts},
	http::request::Parts,
};
use bm_version::{Manager, VersionKey};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::Service;

use super::error::Error;

/// # VersionQuery
/// Query parameters accepted by endpoints that interact with versioned game data.
#[derive(Deserialize, JsonSchema)]
struct VersionQueryParams {
	/// Game version to utilise for this query.
	version: Option<String>,
}

#[derive(OperationIo)]
#[aide(input_with = "Query<VersionQueryParams>")]
pub struct VersionQuery(pub VersionKey);

impl<S> FromRequestParts<S> for VersionQuery
where
	S: Send + Sync,
	Service: FromRef<S>,
{
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let Query(params) = parts
			.extract::<Query<VersionQueryParams>>()
			.await
			.map_err(|error| Error::Invalid(error.to_string()))?;

		let Service { version, .. } = Service::from_ref(state);

		let value = params.version.as_deref().unwrap_or(Manager::DEFAULT_NAME);

		// For backwards compatibilty, we check for a matching name first. There's
		// minimal chance that a name will collide with a key.
		if let Some(version_key) = version.resolve(value) {
			return Ok(Self(version_key));
		}

		// Try parsing the value as a version key. Keys are just hex (which isn't
		// numbers), so we need to double check it's actually a version key before
		// succeeding.
		if let Ok(version_key) = value.parse::<VersionKey>() {
			if let Some(_version) = version.version(version_key) {
				return Ok(Self(version_key));
			}
		}

		// Fall through to a failure
		Err(Error::Invalid(format!("unknown version \"{}\"", value)))
	}
}

#[derive(FromRequestParts, OperationIo)]
#[from_request(via(axum::extract::Path), rejection(Error))]
#[aide(input_with = "axum::extract::Path<T>", json_schema)]
pub struct Path<T>(pub T);

#[derive(FromRequestParts, OperationIo)]
#[from_request(via(axum::extract::Query), rejection(Error))]
#[aide(input_with = "axum::extract::Query<T>", json_schema)]
pub struct Query<T>(pub T);

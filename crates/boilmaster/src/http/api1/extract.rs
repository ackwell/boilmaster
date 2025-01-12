use aide::OperationIo;
use axum::{
	async_trait,
	extract::{FromRef, FromRequestParts},
	http::request::Parts,
	RequestPartsExt,
};
use bm_version::VersionKey;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::http::service::Service;

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

#[async_trait]
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

		let version_name = params.version.as_deref();
		let version_key = version.resolve(version_name).ok_or_else(|| {
			Error::Invalid(format!(
				"unknown version \"{}\"",
				version_name.unwrap_or("(none)")
			))
		})?;

		Ok(Self(version_key))
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

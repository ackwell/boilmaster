use std::convert::Infallible;

use aide::OperationIo;
use axum::{
	async_trait,
	extract::{FromRef, FromRequestParts, OriginalUri},
	http::{request::Parts, Uri},
	RequestPartsExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{http::service::Service, version::VersionKey};

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

// This cursed garbage courtesy of trying to get the path of the parent router. Fun.
pub struct RouterPath(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for RouterPath {
	type Rejection = Infallible;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		let uri = parts.extract::<Uri>().await?;
		let OriginalUri(original_uri) = parts.extract::<OriginalUri>().await?;

		let router_path = original_uri.path().strip_suffix(uri.path()).unwrap_or("");

		Ok(Self(router_path.into()))
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

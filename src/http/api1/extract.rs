use axum::{
	async_trait,
	extract::{FromRef, FromRequestParts},
	http::request::Parts,
	RequestPartsExt,
};
use serde::Deserialize;

use crate::{http::service, version::VersionKey};

use super::error::Error;

#[derive(Deserialize)]
struct VersionQueryParams {
	version: Option<String>,
}

pub struct VersionQuery(pub VersionKey);

#[async_trait]
impl<S> FromRequestParts<S> for VersionQuery
where
	S: Send + Sync,
	service::Version: FromRef<S>,
{
	type Rejection = Error;

	async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
		let Query(params) = parts
			.extract::<Query<VersionQueryParams>>()
			.await
			.map_err(|error| Error::Invalid(error.to_string()))?;

		let version = service::Version::from_ref(state);

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

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(Error))]
pub struct Path<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(Error))]
pub struct Query<T>(pub T);

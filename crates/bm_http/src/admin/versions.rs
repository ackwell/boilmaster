use anyhow::Context;
use axum::{
	Router, debug_handler,
	extract::{NestedPath, OriginalUri, Path, State},
	response::{IntoResponse, Redirect},
	routing::get,
};
use bm_version::{VersionKey, VersionSource};
use maud::{Render, html};

use crate::{http::HttpState, service::Service};

use super::{base::BaseTemplate, error::Result};

pub fn router(state: HttpState) -> Router {
	Router::new()
		.route("/", get(versions))
		.route("/update/{source}", get(update))
		.with_state(state)
}

struct VersionInfo {
	key: VersionKey,
	patch: String,
	names: Vec<String>,
	banned: bool,
}

#[debug_handler(state = HttpState)]
async fn versions(
	OriginalUri(uri): OriginalUri,
	State(Service {
		version: version_service,
		..
	}): State<Service>,
) -> Result<impl IntoResponse> {
	let version_info = |key: VersionKey| -> Result<_> {
		let version = version_service.version(key).context("missing version")?;

		let patch = version
			.repositories
			.first()
			.map(|repository| repository.latest().name.clone())
			.unwrap_or_else(|| "(NONE)".into());

		Ok(VersionInfo {
			key,
			patch,
			names: version_service.names(key).context("missing version")?,
			banned: version.ban_time.is_some(),
		})
	};

	let mut versions = version_service
		.keys()
		.into_iter()
		.map(version_info)
		.collect::<Result<Vec<_>>>()?;

	versions.sort_unstable_by(|a, b| a.patch.cmp(&b.patch).reverse());

	Ok((BaseTemplate {
		title: "versions".to_string(),
		content: html! {
			details.dropdown {
				summary.outline role="button" { "Update" }
				ul {
					li { a href={ (uri) "/update/thaliak" } { "Thaliak v1" } }
					li { a href={ (uri) "/update/thaliak2" } { "Thaliak v2" } }
				}
			}

			table.striped {
				thead {
					tr {
						th { "key" }
						th { "names" }
						th { "banned" }
						th { "patch" }
					}
				}

				tbody {
					@for version in versions {
						tr {
							th {
								code {
									a href={ (uri) "/" (version.key) } {
										(version.key)
									}
								}
							}

							td {
								@for (index, name) in version.names.iter().enumerate() {
									@if index > 0 { ", " }
									(name)
								}
							}

							td {
								@if version.banned {
									"❌"
								}
							}

							td { (version.patch) }
						}
					}
				}
			}
		},
	})
	.render())
}

#[debug_handler(state = HttpState)]
async fn update(
	nested_path: NestedPath,
	Path(version_source): Path<VersionSource>,
	State(Service {
		version: version_service,
		..
	}): State<Service>,
) -> Result<impl IntoResponse> {
	version_service
		.update(version_source)
		.await
		.context("update failed")?;

	Ok(Redirect::to(nested_path.as_str()))
}

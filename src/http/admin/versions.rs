use anyhow::Context;
use axum::{
	debug_handler,
	extract::{OriginalUri, State},
	response::IntoResponse,
	routing::get,
	Router,
};
use maud::{html, Render};

use crate::{
	http::{http::HttpState, service::Service},
	version::VersionKey,
};

use super::{base::BaseTemplate, error::Result};

pub fn router(state: HttpState) -> Router {
	Router::new().route("/", get(versions).with_state(state))
}

struct VersionInfo {
	key: VersionKey,
	patches: Vec<(String, String)>,
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

		let latest = version
			.repositories
			.into_iter()
			.map(|repository| (repository.name, repository.patches.last().name.clone()))
			.collect();

		Ok(VersionInfo {
			key,
			patches: latest,
			names: version_service.names(key).context("missing version")?,
			banned: version.ban_time.is_some(),
		})
	};

	let versions = version_service
		.keys()
		.into_iter()
		.map(version_info)
		.collect::<Result<Vec<_>>>()?;

	Ok((BaseTemplate {
		title: "versions".to_string(),
		content: html! {
			@for version in versions {
				h2 {
					a href={ (uri) "/" (version.key) } {
						(version.key)
					}

					" ("
					@for (index, name) in version.names.iter().enumerate() {
						@if index > 0 { ", " }
						(name)
					}
					")"

					@if version.banned {
						" (banned)"
					}
				}

				dl {
					@for (repository, patch) in &version.patches {
						dt { (repository) }
						dd { (patch) }
					}
				}
			}
		},
	})
	.render())
}

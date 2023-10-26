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
	http::{error::Result, service},
	version3::VersionKey,
};

use super::base::BaseTemplate;

pub fn router() -> Router<service::State> {
	Router::new().route("/", get(versions))
}

struct VersionInfo {
	key: VersionKey,
	patches: Vec<(String, String)>,
	names: Vec<String>,
}

#[debug_handler]
async fn versions(
	OriginalUri(uri): OriginalUri,
	State(version): State<service::Version>,
) -> Result<impl IntoResponse> {
	let version_info = |key: VersionKey| -> Result<_> {
		let latest = version
			.version(key)
			.context("missing version")?
			.repositories
			.into_iter()
			.map(|repository| (repository.name, repository.patches.last().name.clone()))
			.collect();

		Ok(VersionInfo {
			key,
			patches: latest,
			names: version.names(key).context("missing version")?,
		})
	};

	let versions = version
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

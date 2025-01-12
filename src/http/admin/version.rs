use std::collections::HashSet;

use anyhow::Context;
use axum::{
	debug_handler,
	extract::{OriginalUri, Path, State},
	response::{IntoResponse, Redirect},
	routing::get,
	Form, Router,
};
use maud::{html, Render};
use serde::Deserialize;

use crate::{
	http::{http::HttpState, service::Service},
	version::VersionKey,
};

use super::{base::BaseTemplate, error::Result};

pub fn router(state: HttpState) -> Router {
	Router::new()
		.route(
			"/{version_key}",
			get(get_version)
				.post(post_version)
				.with_state(state.clone()),
		)
		.route(
			"/{version_key}/delete",
			get(delete_instructions).with_state(state),
		)
}

#[debug_handler(state = HttpState)]
async fn get_version(
	OriginalUri(uri): OriginalUri,
	Path(version_key): Path<VersionKey>,
	State(Service {
		version: version_service,
		..
	}): State<Service>,
) -> Result<impl IntoResponse> {
	let version = version_service
		.version(version_key)
		.context("unknown version")?;

	let banned = version.ban_time.is_some();

	let names = version_service
		.names(version_key)
		.context("unknown version")?;

	// Patches are stored in oldest-first order for IW, which is lovely in code
	// and horrible for reading. Given this is ostensibly the reading bit of the
	// application, fix that.
	let patch_list = version
		.repositories
		.into_iter()
		.map(|repository| {
			(
				repository.name,
				repository.patches.into_iter().rev().collect::<Vec<_>>(),
			)
		})
		.collect::<Vec<_>>();

	Ok((BaseTemplate {
		title: format!("version {}", version_key),
		content: html! {
			form action=(uri) method="post" {
				fieldset {
					label {
						"names"
						input type="text" id="names" name="names" value={
							@for (index, name) in names.into_iter().enumerate() {
								@if index > 0 { ", " }
								(name)
							}
						};
						small { "comma separated" }
					}

					label {
						input
							type="checkbox"
							role="switch"
							name="ban"
							value="true"
							checked[banned]
							style="--pico-switch-checked-background-color: #AF291D"; // pico-color-red-600

						"banned"
					}
				}

				button type="submit" { "save" };
			}

			h2 { "patches" }
			@for (repository, patches) in patch_list {
				details {
					summary {
						(repository)
						" ("
						(patches.len()) " patches, "
						"latest: " (patches.first().map(|patch| patch.name.as_str()).unwrap_or("none"))
						")"
					}
					ul {
						@for patch in patches {
							li { (patch.name) }
						}
					}
				}
			}
		},
	})
	.render())
}

#[derive(Debug, Deserialize)]
struct VersionPostRequest {
	names: String,
	// Will only be set if checked.
	ban: Option<bool>,
}

#[debug_handler(state = HttpState)]
async fn post_version(
	OriginalUri(uri): OriginalUri,
	Path(version_key): Path<VersionKey>,
	State(Service { version, .. }): State<Service>,
	Form(request): Form<VersionPostRequest>,
) -> Result<impl IntoResponse> {
	let names = request.names.split(',').map(str::trim);
	version.set_names(version_key, names).await?;

	let banned = request.ban.is_some();
	version.set_banned(version_key, banned).await?;

	Ok(Redirect::to(&uri.to_string()))
}

#[debug_handler(state = HttpState)]
async fn delete_instructions(
	Path(target_key): Path<VersionKey>,
	State(Service { version, .. }): State<Service>,
) -> Result<impl IntoResponse> {
	// Build set of each repository's patch paths for the target version.
	let target = version.version(target_key).context("unknown version")?;
	let mut target_paths = target
		.repositories
		.into_iter()
		.flat_map(|repository| repository.patches.into_iter().map(|patch| patch.path))
		.collect::<HashSet<_>>();

	// Iterate over all versions other than the target.
	let keys = version.keys().into_iter().filter(|&key| key != target_key);
	for key in keys {
		let other = version.version(key).context("missing version")?;

		// Remove any paths in this versions's repositories.
		for repository in other.repositories {
			for patch in repository.patches {
				target_paths.remove(&patch.path);
			}
		}
	}

	Ok((BaseTemplate {
		title: format!("version {target_key}: delete instructions"),
		content: html! {
			h2 { "instructions" }
			ol {
				li { "copy the list of orphaned patch files below" }
				li { "delete " code { (version.version_path(target_key).to_string_lossy()) } }
				li {
					"remove all references to "
					code { (target_key) }
					" from "
					code { (version.metadata_path().to_string_lossy()) }
				}
				li { "restart service and ensure that " code { (target_key) } " is not listed" }
				li { "delete orphaned patch files" }
			}
			h2 { "orphaned patch files" }
			ul {
				@for path in target_paths {
					li { (path.to_string_lossy()) }
				}
			}
		},
	})
	.render())
}

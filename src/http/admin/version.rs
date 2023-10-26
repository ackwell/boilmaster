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
	http::{error::Result, service},
	version3::VersionKey,
};

use super::base::BaseTemplate;

pub fn router() -> Router<service::State> {
	Router::new().route("/:version_key", get(get_version).post(post_version))
}

#[debug_handler]
async fn get_version(
	OriginalUri(uri): OriginalUri,
	Path(version_key): Path<VersionKey>,
	State(version): State<service::Version>,
) -> Result<impl IntoResponse> {
	let names = version.names(version_key).context("unknown version")?;

	// Patches are stored in oldest-first order for IW, which is lovely in code
	// and horrible for reading. Given this is ostensibly the reading bit of the
	// application, fix that.
	let patch_list = version
		.version(version_key)
		.context("unknown version")?
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
			h2 { "names" }
			form action=(uri) method="post" {
				input type="text" name="names" value={
					@for (index, name) in names.into_iter().enumerate() {
						@if index > 0 { ", " }
						(name)
					}
				};
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
}

#[debug_handler]
async fn post_version(
	OriginalUri(uri): OriginalUri,
	Path(version_key): Path<VersionKey>,
	State(version): State<service::Version>,
	Form(request): Form<VersionPostRequest>,
) -> Result<impl IntoResponse> {
	let names = request.names.split(',').map(str::trim);
	version.set_names(version_key, names).await?;

	Ok(Redirect::to(&uri.to_string()))
}

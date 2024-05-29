use std::{
	collections::HashMap,
	fs,
	io::{self, Write},
	path::{Path, PathBuf},
	sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use figment::value::magic::RelativePathBuf;
use serde::Deserialize;
use tokio::sync::{broadcast, Semaphore};

use super::{thaliak, version};

enum State {
	Pending(broadcast::Receiver<version::Patch>),
	Available(version::Patch),
}

#[derive(Debug, Deserialize)]
pub struct Config {
	directory: RelativePathBuf,
	concurrency: usize,
	user_agent: String,
}

pub struct Patcher {
	directory: PathBuf,
	semaphore: Arc<Semaphore>,
	client: reqwest::Client,
	patch_states: Arc<Mutex<HashMap<PathBuf, State>>>,
}

impl Patcher {
	pub fn new(config: Config) -> Self {
		Self {
			directory: config.directory.relative(),
			semaphore: Arc::new(Semaphore::new(config.concurrency)),
			client: reqwest::Client::builder()
				.user_agent(config.user_agent)
				.build()
				.expect("failed to build reqwest client"),
			patch_states: Default::default(),
		}
	}

	pub fn patch_path(&self, repository: &str, patch: &str) -> PathBuf {
		self.directory.join(repository).join(patch)
	}

	pub async fn to_local_patch(
		&self,
		repository: &str,
		thaliak_patch: thaliak::Patch,
	) -> Result<version::Patch> {
		let patch_path = self.patch_path(repository, &thaliak_patch.name);

		// TODO: It seems wasteful to call this hundreds of times every update when it'll do something less than 10 times ever.
		let repository_directory = patch_path
			.parent()
			.expect("patches should always be within a folder");
		fs::create_dir_all(&repository_directory)
			.with_context(|| format!("failed to create directory {repository_directory:?}"))?;

		let mut patch_states = self.patch_states.lock().expect("poisoned");

		let patch = match patch_states.get(&patch_path) {
			// Patch is already known to be available.
			Some(State::Available(patch)) => patch.clone(),

			// Another task has taken ownership of this patch. Subscribe to the
			// notification channel, then release the lock to allow other tasks to
			// work while waiting for the go-ahead.
			Some(State::Pending(rx)) => {
				let mut receiver = rx.resubscribe();
				drop(patch_states);
				receiver.recv().await?
			}

			// Patch isn't yet known, take ownership for validating/obtaining the patch.
			None => {
				// Set up a notification channel in case any other channel is interested
				// in this patch, then release lock while handling the patch itself.
				let (tx, rx) = broadcast::channel(1);
				patch_states.insert(patch_path.clone(), State::Pending(rx));
				drop(patch_states);

				let patch = self
					.maybe_download_patch(thaliak_patch, patch_path.clone())
					.await?;

				// Download is complete - relock to insert, and broadcast the value to
				// any waiting consumers. We don't care if the notification is successful,
				// it's fairly common there will be no other tasks interested in a given patch.
				self.patch_states
					.lock()
					.expect("poisoned")
					.insert(patch_path, State::Available(patch.clone()));
				let _ = tx.send(patch.clone());

				patch
			}
		};

		Ok(patch)
	}

	async fn maybe_download_patch(
		&self,
		thaliak_patch: thaliak::Patch,
		patch_path: PathBuf,
	) -> Result<version::Patch> {
		let patch_name = thaliak_patch.name.clone();

		// If we need to fetch the patch, wait for a permit then spin off a task to handle the download.
		if self.should_fetch_patch(&thaliak_patch, &patch_path)? {
			let permit = self.semaphore.clone().acquire_owned().await.unwrap();

			let client = self.client.clone();
			let patch_path = patch_path.clone();
			let handle = tokio::spawn(async move {
				let result = fetch_patch(client, &thaliak_patch, &patch_path).await;
				drop(permit);
				result
			});
			handle.await??;
		}

		let patch = version::Patch {
			name: patch_name,
			path: patch_path,
		};

		Ok(patch)
	}

	fn should_fetch_patch(&self, patch: &thaliak::Patch, path: &Path) -> Result<bool> {
		// If the file doesn't exist, we'll need to download it.
		let metadata = match path.metadata() {
			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
			other => other?,
		};

		// If it's not a file, there's definately a problem.
		if !metadata.is_file() {
			anyhow::bail!("patch path {path:?} exists but is not a file");
		}

		// If there's a size mismatch, we should re-fetch (likely a partial download).
		if metadata.len() != patch.size {
			tracing::warn!(
			  patch = %patch.name,
			  expected = patch.size,
			  got = metadata.len(),
			  "size mismatch, will re-fetch"
			);
			return Ok(true);
		}

		// Otherwise, we can assume the file is what we want.
		Ok(false)
	}
}

#[tracing::instrument(level = "info", skip_all, fields(url = patch.url))]
async fn fetch_patch(client: reqwest::Client, patch: &thaliak::Patch, path: &Path) -> Result<()> {
	tracing::info!("fetching patch");

	// Create the target file before opening any connections.
	let mut target_file = fs::File::create(path)?;

	// TODO: both of the below failure conditions may be worth retrying over? consider.

	// Initiate the request for the patch file. If there's a non-success status,
	// we've got an issue and should fail fast.
	let mut response = client.get(&patch.url).send().await?.error_for_status()?;

	// If there's a mismatch on content-length, there's something wrong with this url.
	let content_length = response
		.content_length()
		.ok_or_else(|| anyhow::anyhow!("no content-length supplied for {}", patch.url))?;

	if content_length != patch.size {
		anyhow::bail!(
			"unexpected content-length: expected {}, got {content_length}",
			patch.size
		)
	}

	// Stream the response body to disk.
	let mut position = 0;
	let mut last_report = 0.0;

	while let Some(chunk) = response.chunk().await? {
		// This is blocking - is it worth trying to use async fs, or is the slowdown from that going to be Problematic:tm:?
		target_file.write_all(&chunk)?;

		position += u64::try_from(chunk.len()).unwrap();
		let report_pos = f64::round((position as f64 / content_length as f64) * 20.0) * 5.0;
		if report_pos > last_report {
			tracing::debug!("{position}/{content_length} ({report_pos}%)");
			last_report = report_pos;
		}
	}

	Ok(())
}

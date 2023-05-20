use crate::version::VersionKey;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unknown version {0}")]
	UnknownVersion(VersionKey),

	#[error("version {0} is not ready yet")]
	PendingVersion(VersionKey),

	#[error("unknown language \"{0}\"")]
	UnknownLanguage(String),

	#[error(transparent)]
	Failure(#[from] anyhow::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

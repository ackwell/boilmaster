use crate::version::VersionKey;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unknown version {0}")]
	UnknownVersion(VersionKey),

	#[error(transparent)]
	Failure(#[from] anyhow::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

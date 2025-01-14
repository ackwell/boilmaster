#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unknown schema source \"{0}\"")]
	UnknownSource(String),

	#[error("invalid schema version \"{0}\"")]
	InvalidVersion(String),

	#[error(transparent)]
	Failure(#[from] anyhow::Error),
}

impl From<ironworks_schema::Error> for Error {
	fn from(error: ironworks_schema::Error) -> Self {
		use ironworks_schema::Error as SE;
		use ironworks_schema::ErrorValue as SEV;
		match error {
			SE::NotFound(SEV::Version(version)) => Error::InvalidVersion(version.into()),
			other => Error::Failure(other.into()),
		}
	}
}

macro_rules! impl_to_failure {
	($source:ty) => {
		impl From<$source> for Error {
			fn from(value: $source) -> Self {
				Self::Failure(value.into())
			}
		}
	};
}

impl_to_failure!(bm_data::Error);
impl_to_failure!(ironworks::Error);

pub type Result<T, E = Error> = std::result::Result<T, E>;

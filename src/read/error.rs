#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The requested resource could not be found.
	#[error("{0}")]
	NotFound(String),

	/// The provided filter does not map cleanly onto the sheet schema.
	#[error("filter <-> schema mismatch on {}: {}", .0.field, .0.reason)]
	FilterSchemaMismatch(MismatchError),

	/// The sheet schema does not map cleanly onto the underlying game data.
	#[error("schema <-> game mismatch on {}: {}", .0.field, .0.reason)]
	SchemaGameMismatch(MismatchError),

	#[error(transparent)]
	Failure(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct MismatchError {
	pub(super) field: String,
	pub(super) reason: String,
}

impl From<ironworks::Error> for Error {
	fn from(error: ironworks::Error) -> Self {
		use ironworks::Error as IE;
		use ironworks::ErrorValue as EV;
		match error {
			// excel-specific NotFound are relevant for reading, but others are technically failures.
			IE::NotFound(EV::Sheet(..) | EV::Row { .. }) => Error::NotFound(error.to_string()),
			other => Error::Failure(other.into()),
		}
	}
}

impl From<ironworks_schema::Error> for Error {
	fn from(error: ironworks_schema::Error) -> Self {
		use ironworks_schema::Error as ISE;
		use ironworks_schema::ErrorValue as ISEV;
		match error {
			ISE::NotFound(ISEV::Sheet(_)) => Error::NotFound(error.to_string()),
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

impl_to_failure!(std::num::TryFromIntError);

pub type Result<T, E = Error> = std::result::Result<T, E>;

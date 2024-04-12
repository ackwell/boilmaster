#[derive(Debug, thiserror::Error)]
pub enum Error {
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
impl_to_failure!(ironworks::Error);
impl_to_failure!(ironworks_schema::Error);

pub type Result<T, E = Error> = std::result::Result<T, E>;

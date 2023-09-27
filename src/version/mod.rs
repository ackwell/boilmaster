mod key;
mod manager;
mod patch;
mod thaliak;
mod version;

pub use {
	key::VersionKey,
	manager::{Config, Manager},
	patch::Patch,
};

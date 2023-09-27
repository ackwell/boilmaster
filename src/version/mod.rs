mod key;
mod manager;
mod patch;
mod thaliak;

pub use {
	key::VersionKey,
	manager::{Config, Manager},
	patch::Patch,
};

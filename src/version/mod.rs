mod key;
mod manager;
mod patcher;
mod thaliak;
mod version;

pub use {
	key::VersionKey,
	manager::{Config, Manager},
	version::{Patch, Repository, Version},
};

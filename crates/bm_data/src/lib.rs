#![allow(clippy::new_without_default)]

mod data;
mod error;

pub use {
	data::{Data, Version},
	error::Error,
};

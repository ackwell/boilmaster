mod error;
mod exdschema;
mod provider;
mod specifier;

pub use {
	error::Error,
	provider::{Config, Provider},
	specifier::{CanonicalSpecifier, Specifier},
};

use std::{io::Cursor, path::Path};

use anyhow::Context;
use image::ImageFormat;

use crate::data;

use super::{
	error::{Error, Result},
	format::Format,
	texture,
};

pub trait Converter {
	// TODO: Consider using a stream for this - the only converter I actually have right now doesn't operate with streams, but it may be relevant for other converters - or possibly would tie in with caching. Ref. https://github.com/tokio-rs/axum/discussions/608 re: responding to requests with streams.
	fn convert(&self, data: &data::Version, path: &str, format: Format) -> Result<Vec<u8>>;
}

pub struct Image;

impl Converter for Image {
	fn convert(&self, data: &data::Version, path: &str, format: Format) -> Result<Vec<u8>> {
		let extension = Path::new(path)
			.extension()
			.and_then(|extension| extension.to_str());

		// TODO: add error handling case on this once more than one format exists.
		let output_format = match format {
			Format::Png => ImageFormat::Png,
		};

		// TODO: should i just pass IW to convert? is there any realistic expectation that a converter will need excel?
		let ironworks = data.ironworks();

		let buffer = match extension {
			Some("tex") | Some("atex") => texture::read(&ironworks, path),

			other => {
				return Err(Error::InvalidConversion(
					other.unwrap_or("(none)").into(),
					format,
				));
			}
		}?;

		// TODO: are there any non-failure cases here?
		let mut bytes = Cursor::new(vec![]);
		buffer
			.write_to(&mut bytes, output_format)
			.context("failed to write output buffer")?;

		Ok(bytes.into_inner())
	}
}

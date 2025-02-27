use std::sync::Arc;

use anyhow::{Context, anyhow};
use bm_version::VersionKey;
use image::{ImageBuffer, Pixel, Rgb};
use ironworks::Ironworks;

use super::{
	error::{Error, Result},
	format::Format,
	texture,
};

pub struct Service {
	data: Arc<bm_data::Data>,
}

impl Service {
	pub fn new(data: Arc<bm_data::Data>) -> Self {
		Self { data }
	}

	pub fn ready(&self) -> bool {
		// No warmup in this service, we're always ready.
		true
	}

	pub fn convert(&self, version: VersionKey, path: &str, format: Format) -> Result<Vec<u8>> {
		// TODO: presumably this is where caching would be resolved

		let data_version = self
			.data
			.version(version)
			.with_context(|| format!("data for {version} not ready"))?;

		let converter = format.converter();
		converter.convert(&data_version, path, format)
	}

	pub fn map(&self, version: VersionKey, territory: &str, index: &str) -> Result<Vec<u8>> {
		let version = self
			.data
			.version(version)
			.with_context(|| format!("data for {version} not ready"))?;

		let ironworks = version.ironworks();

		let image = self.compose_map(&ironworks, territory, index)?;

		texture::write(image, image::ImageFormat::Jpeg)
	}

	fn compose_map(
		&self,
		ironworks: &Ironworks,
		territory: &str,
		index: &str,
	) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
		let path = format!("ui/map/{territory}/{index}/{territory}{index}");
		let mut buffer_map = texture::read(&ironworks, &format!("{path}_m.tex"))?.into_rgb8();

		let buffer_background = match texture::read(&ironworks, &format!("{path}m_m.tex")) {
			// If the background texture wasn't found, we can assume the map texture is pre-composed.
			Err(Error::NotFound(_)) => return Ok(buffer_map),
			Ok(image) => image.into_rgb8(),
			Err(error) => Err(error)?,
		};

		if buffer_map.dimensions() != buffer_background.dimensions() {
			return Err(anyhow!("map and background dimensions differ").into());
		}

		// Multiply the pixels together.
		for (x, y, pixel_map) in buffer_map.enumerate_pixels_mut() {
			let pixel_background = buffer_background.get_pixel(x, y);
			pixel_map.apply2(pixel_background, |a, b| ((a as u32 * b as u32) / 255) as u8)
		}

		Ok(buffer_map)
	}
}

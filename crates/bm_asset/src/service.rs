use std::sync::Arc;

use anyhow::{Context, anyhow};
use bm_version::VersionKey;
use image::{GenericImageView, ImageBuffer, Pixel, Rgba};
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

	pub fn map(
		&self,
		version: VersionKey,
		territory: &str,
		index: &str,
		format: Format,
	) -> Result<Vec<u8>> {
		let version = self
			.data
			.version(version)
			.with_context(|| format!("data for {version} not ready"))?;

		let output_format = match format {
			Format::Jpeg => image::ImageFormat::Jpeg,
			Format::Png => image::ImageFormat::Png,
			Format::Webp => image::ImageFormat::WebP,
		};

		let ironworks = version.ironworks();

		let image = self.compose_map(&ironworks, territory, index)?;

		texture::write(image, output_format)
	}

	fn compose_map(
		&self,
		ironworks: &Ironworks,
		territory: &str,
		index: &str,
	) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
		let path = format!("ui/map/{territory}/{index}/{territory}{index}");
		let mut buffer_map = texture::read(&ironworks, &format!("{path}_m.tex"))?.into_rgba8();

		let buffer_background = match texture::read(&ironworks, &format!("{path}m_m.tex")) {
			// If the background texture wasn't found, we can assume the map texture is pre-composed.
			Err(Error::NotFound(_)) => return Ok(buffer_map),
			Ok(image) => {
				// Some maps have a fully black & transparent `m` texture and are pre-composited.
				// A pixel from the center of the texture is checked since some maps have a transparent border.
				let dimensions = image.dimensions();
				if image
					.get_pixel(dimensions.0 / 2, dimensions.1 / 2)
					.channels()
					.iter()
					.all(|c| *c == 0)
				{
					return Ok(buffer_map);
				}
				image.into_rgba8()
			}
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

// TODO: Ironworks doesn't currently expose a way to check if a file exists
// without reading it into a file structure as well. We're working around this
// by making it "read" nothing - successfully constructing this necessitates
// that the file could be found.
#[derive(Debug)]
struct FileExists;
impl ironworks::file::File for FileExists {
	fn read(_stream: impl ironworks::FileStream) -> Result<Self, ironworks::Error> {
		Ok(Self)
	}
}

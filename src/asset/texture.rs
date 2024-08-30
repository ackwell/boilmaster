use anyhow::Context;
use image::{DynamicImage, ImageBuffer};
use ironworks::{file::tex, Ironworks};
use itertools::Itertools;

use super::error::{Error, Result};

pub fn read(ironworks: &Ironworks, path: &str) -> Result<DynamicImage> {
	let texture = match ironworks.file::<tex::Texture>(path) {
		Ok(value) => value,
		Err(ironworks::Error::NotFound(_)) => return Err(Error::NotFound(path.into())),
		other => other.context("read file")?,
	};

	if !matches!(texture.kind(), tex::TextureKind::D2) {
		return Err(Error::UnsupportedSource(
			path.into(),
			format!("unhandled texture dimension {:?}", texture.kind()),
		));
	}

	let buffer = match texture.format() {
		tex::Format::A8 => read_a8(texture)?,

		tex::Format::Rgba4 => read_rgba4(texture)?,
		tex::Format::Rgb5a1 => read_rgb5a1(texture)?,
		tex::Format::Argb8 => read_argb8(texture)?,

		tex::Format::Dxt1 => read_texture_dxt(texture, texpresso::Format::Bc1)?,
		tex::Format::Dxt3 => read_texture_dxt(texture, texpresso::Format::Bc2)?,
		tex::Format::Dxt5 => read_texture_dxt(texture, texpresso::Format::Bc3)?,

		other => {
			return Err(Error::UnsupportedSource(
				path.into(),
				format!("unhandled texture format {other:?}"),
			))
		}
	};

	Ok(buffer)
}

fn read_a8(texture: tex::Texture) -> Result<DynamicImage> {
	let buffer = ImageBuffer::from_raw(
		texture.width().into(),
		texture.height().into(),
		texture.data().to_owned(),
	)
	.context("failed to build image buffer")?;
	Ok(DynamicImage::ImageLuma8(buffer))
}

// NOTE: This is actually BGRA4. Need to update names in ironworks. Reference https://github.com/NotAdam/Lumina/blob/master/src/Lumina/Data/Files/TexFile.cs#L53.
fn read_rgba4(texture: tex::Texture) -> Result<DynamicImage> {
	let data = texture
		.data()
		.iter()
		.tuples()
		.flat_map(|(gb, ar)| {
			let b = (gb & 0x0F) * 0x11;
			let g = (gb >> 4) * 0x11;
			let r = (ar & 0x0F) * 0x11;
			let a = (ar >> 4) * 0x11;
			[r, g, b, a]
		})
		.collect::<Vec<_>>();

	let buffer = ImageBuffer::from_raw(texture.width().into(), texture.height().into(), data)
		.context("failed to build image buffer")?;
	Ok(DynamicImage::ImageRgba8(buffer))
}

fn read_rgb5a1(texture: tex::Texture) -> Result<DynamicImage> {
	let data = texture
		.data()
		.iter()
		.tuples()
		.flat_map(|(b, a)| {
			let pixel = u16::from(*b) | (u16::from(*a) << 8);
			let r = (pixel & 0x7C00) >> 7;
			let g = (pixel & 0x03E0) >> 2;
			let b = (pixel & 0x001F) << 3;
			let a = ((pixel & 0x8000) >> 15) * 0xFF;
			[r, g, b, a]
		})
		.map(|value| u8::try_from(value).unwrap())
		.collect::<Vec<_>>();

	let buffer = ImageBuffer::from_raw(texture.width().into(), texture.height().into(), data)
		.context("failed to build image buffer")?;
	Ok(DynamicImage::ImageRgba8(buffer))
}

fn read_argb8(texture: tex::Texture) -> Result<DynamicImage> {
	// TODO: seems really wasteful to copy the entire image in memory just to reassign the channels. think of a better way to do this.
	// TODO: use array_chunks once it hits stable
	let data = texture
		.data()
		.iter()
		.tuples()
		.flat_map(|(b, g, r, a)| [r, g, b, a])
		.copied()
		.collect::<Vec<_>>();

	let buffer = ImageBuffer::from_raw(texture.width().into(), texture.height().into(), data)
		.context("failed to build image buffer")?;
	Ok(DynamicImage::ImageRgba8(buffer))
}

fn read_texture_dxt(texture: tex::Texture, dxt_format: texpresso::Format) -> Result<DynamicImage> {
	let width = usize::from(texture.width());
	let height = usize::from(texture.height());

	let mut dxt_buffer = vec![0; width * height * 4];
	dxt_format.decompress(texture.data(), width, height, &mut dxt_buffer);

	let image_buffer = ImageBuffer::from_raw(
		width.try_into().unwrap(),
		height.try_into().unwrap(),
		dxt_buffer,
	)
	.context("failed to build image buffer")?;
	Ok(DynamicImage::ImageRgba8(image_buffer))
}

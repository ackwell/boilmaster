use std::sync::OnceLock;

use aho_corasick::AhoCorasick;
use anyhow::Context;
use ironworks::{
	excel::Excel,
	sestring::{
		Error as SeStringError, SeString,
		format::{Color, ColorUsage, Input, Write, format},
	},
};

use super::error::Result;

pub fn build_input(excel: &Excel) -> Result<Input> {
	let mut input = Input::new();

	// Someone put a bullet through me if I need to make this configurable.
	let sheet = excel
		.sheet("UIColor")
		.context("failed to read UIColor sheet")?;

	for row in sheet.into_iter() {
		let [r, g, b, a] = row
			.field(0)
			.ok()
			.and_then(|field| field.into_u32().ok())
			.context("failed to read color field")?
			.to_be_bytes();

		input.add_color(ColorUsage::Foreground, row.row_id(), Color { r, g, b, a });
	}

	Ok(input)
}

pub fn as_html(string: SeString, input: &Input) -> Result<String, SeStringError> {
	let mut writer = HtmlWriter::default();
	format(string, input, &mut writer)?;
	Ok(writer.buffer)
}

#[derive(Debug, Default)]
struct HtmlWriter {
	buffer: String,
}

impl Write for HtmlWriter {
	fn write_str(&mut self, str: &str) -> Result<(), SeStringError> {
		// Probaly overkill for this but it'll be nice if I add more replacements.
		static PATTERN: OnceLock<AhoCorasick> = OnceLock::new();
		let pattern = PATTERN.get_or_init(|| {
			AhoCorasick::new(["\n"]).expect("pattern construction should not fail")
		});

		let output = pattern.replace_all(str, &["<br>"]);

		self.buffer.push_str(&output);
		Ok(())
	}

	// Only paying attention to foreground for now. Anything more involved will
	// need additional tracking for stacks and a lot more spans.

	fn push_color(&mut self, usage: ColorUsage, color: Color) -> Result<(), SeStringError> {
		if usage != ColorUsage::Foreground {
			return Ok(());
		}

		let Color { r, g, b, a } = color;
		let a = f32::from(a) / 255.;
		self.buffer
			.push_str(&format!(r#"<span style="color:rgba({r},{g},{b},{a});">"#));

		Ok(())
	}

	fn pop_color(&mut self, usage: ColorUsage) -> Result<(), SeStringError> {
		if usage != ColorUsage::Foreground {
			return Ok(());
		}

		self.buffer.push_str("</span>");

		Ok(())
	}
}

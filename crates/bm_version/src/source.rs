use serde::Deserialize;

#[derive(Debug)]
pub struct Patch {
	pub name: String,
	pub url: String,
	pub size: u64,
	// TODO: hash
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
	Thaliak,
	Thaliak2,
}

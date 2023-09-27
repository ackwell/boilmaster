use std::collections::HashMap;

use super::patch::Patch;

// TODO: Need some sort of status enum here so I can flag versions as unreachable &c
#[derive(Debug)]
pub struct Version {
	// TODO: does this even need to be a hashmap any more - I doubt i'll be persisting this, so it's mapping to repositories is kind of a runtime given
	patches: HashMap<String, Vec<Patch>>,
}

impl Version {
	pub fn new() -> Self {
		Self {
			patches: Default::default(),
		}
	}

	pub fn patches(&self) -> &HashMap<String, Vec<Patch>> {
		&self.patches
	}

	pub fn update(&mut self, patches: HashMap<String, Vec<Patch>>) {
		self.patches = patches;
	}
}

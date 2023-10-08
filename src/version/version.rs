use super::patch::Patch;

#[derive(Debug, Clone)]
pub enum Version {
	Available(PatchList),

	Unavailable(Vec<(String, String, Status)>),
}

pub type PatchList = Vec<(String, Vec<Patch>)>;

#[derive(Debug, Clone)]
pub enum Status {
	Ok,
	Unresolved(String),
}

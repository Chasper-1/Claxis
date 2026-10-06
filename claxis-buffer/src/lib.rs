mod arena;
mod buffer;
mod current;
mod edit;
mod history;
mod segment;

pub use arena::{AddArena, AddId};
pub use buffer::Buffer;
pub use current::Current;
pub use edit::{Edit, Error};
pub use history::{History, Record};
pub use segment::{Segment, Source};

#[cfg(test)]
mod tests;

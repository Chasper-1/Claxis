mod arena;
mod buffer;
mod current;
mod edit;
mod history;
mod segment;

pub use arena::{ARENA_CAPACITY, AddArena, AddId};
pub use buffer::{Buffer, Snapshot};
pub use current::Current;
pub use edit::{Edit, Error};
pub use history::{History, Record};
pub use segment::{Segment, Source};

#[cfg(test)]
mod tests;

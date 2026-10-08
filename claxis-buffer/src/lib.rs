mod arena;
mod buffer;
mod current;
mod edit;
mod history;
mod segment;

pub use arena::{ARENA_CAPACITY, AddArena, Kind, Record, RecordId};
pub use buffer::Buffer;
pub use current::Current;
pub use edit::{Edit, Error};
pub use history::History;
pub use segment::{ADDED, ORIGINAL, Segment};

#[cfg(test)]
mod tests;

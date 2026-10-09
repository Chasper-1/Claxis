mod arena;
mod buffer;
mod current;
mod segment;

pub use arena::{Arena, ArenaSize, ORIGINAL_ID, Record, RecordId};
pub use buffer::{Buffer, Error};
pub use current::Current;
pub use segment::{ADDED, ORIGINAL, Segment, TextRef};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod invariants;

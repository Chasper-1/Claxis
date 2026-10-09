mod arena;
mod buffer;
mod current;
pub mod defaults;
mod messages;
mod segment;

pub use arena::{Arena, DEFAULT_DEPTH, MAX_DEPTH, ORIGINAL_ID, Record, RecordId};
pub use buffer::{Buffer, Error};
pub use current::Current;
pub use messages::{En, Messages, ParseError, substitute};
pub use segment::{ADDED, ORIGINAL, Segment, TextRef};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod invariants;

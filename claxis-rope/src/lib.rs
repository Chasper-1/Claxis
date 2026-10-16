mod arena;
mod buffer;
mod current;
pub mod defaults;
pub mod history;
pub mod messages;
mod segment;
pub mod snapshot;

pub use arena::{Arena, DEFAULT_DEPTH, MAX_DEPTH, ORIGINAL_ID, Record, RecordId};
pub use buffer::{Buffer, Error};
pub use current::Current;
pub use history::History;
pub use messages::{En, Messages, ParseError, substitute};
pub use segment::{ADDED, ORIGINAL, Segment, TextRef};
pub use snapshot::{NullSink, SnapshotSink};

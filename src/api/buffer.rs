//! Ручки крейта `claxis-buffer`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: документ,
//! его правки, история, снятая в байты, и каталог сообщений об ошибках.

pub use claxis_buffer::commands::COMMANDS;
pub use claxis_buffer::defaults;
pub use claxis_buffer::messages;
pub use claxis_buffer::{
    DEFAULT_DEPTH, Document, En, Error, History, MIN_DEPTH, Memory, Messages, NullSink, ParseError,
    Record, Result, SnapshotSink, substitute,
};

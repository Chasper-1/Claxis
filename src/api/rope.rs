//! Ручки крейта `claxis-rope`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: документ,
//! его правки, история, снятая в байты, и каталог сообщений об ошибках.

pub use claxis_rope::commands::COMMANDS;
pub use claxis_rope::defaults;
pub use claxis_rope::messages;
pub use claxis_rope::{
    DEFAULT_DEPTH, Document, En, Error, History, MIN_DEPTH, Memory, Messages, NullSink, ParseError,
    Record, Result, SnapshotSink, substitute,
};

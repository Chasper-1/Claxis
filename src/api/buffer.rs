//! Ручки крейта `claxis-buffer`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: типы и
//! функции. Реализация лежит в самом крейте, а связь с ручками других
//! крейтов делает маршрутизатор `crate::router`.

pub use claxis_buffer::defaults::buffer::HISTORY_DEPTH;
pub use claxis_buffer::messages;
pub use claxis_buffer::{
    Buffer, DEFAULT_DEPTH, En as MessagesEn, Error, MAX_DEPTH, Messages, NullSink, ORIGINAL_ID,
    ParseError, Record, RecordId, Segment, SnapshotSink, substitute,
};

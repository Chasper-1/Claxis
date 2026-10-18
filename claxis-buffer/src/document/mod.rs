//! Документ: текст, история и снапшоты.
//!
//! Функционал тот же, что у `claxis-buffer`: вставка, удаление, замена,
//! отмена, повтор, чтение, снапшот. Отличается только устройство: текст лежит
//! в одном буфере, а лист дерева — диапазон в нём, поэтому лист не
//! привязан к границам правок и одна вставка даёт один лист, а не три.

mod core;
mod edit;
mod error;
mod record;
mod snapshot;

pub use core::Document;
pub use error::{DEFAULT_DEPTH, Error, MIN_DEPTH, Memory, Result, SnapshotSink};
pub use record::{Kind, Record};

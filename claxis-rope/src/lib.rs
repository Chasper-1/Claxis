//! Rope: текст как последовательность диапазонов одного буфера.
//!
//! Отличие от дерева с записями: лист здесь `start, len`, и он не привязан к
//! границам правок. Поэтому вставка в середину не разрезает старые листья и
//! даёт один новый, а не три. Функционал тот же, что у `claxis-buffer`:
//! вставка, удаление, замена, отмена, повтор, чтение, снапшот.

pub mod commands;
pub mod defaults;
pub mod document;
pub mod history;
pub mod messages;
pub mod node;
pub mod tree;

pub use document::{
    DEFAULT_DEPTH, Document, Error, MIN_DEPTH, Memory, NullSink, Record, Result, SnapshotSink,
};
pub use history::History;
pub use messages::{En, Messages, ParseError, substitute};
pub use node::{FREE, Leaf, NIL, Node};
pub use tree::{Sink, Tree};

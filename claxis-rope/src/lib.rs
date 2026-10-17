//! Rope: текст как дерево листьев над общим буфером.
//!
//! Лист — диапазон общего буфера. Размер чанка зависит от объёма файла. Лист
//! делится, если тянет больше двух чанков, соседние листья сливаются, если
//! влезают в два. Дерево сбалансировано по весу: вес узла — число байт под ним.

pub mod chunk;
pub mod leaf;
pub mod node;
pub mod pool;
pub mod tree;

pub use chunk::chunk_size;
pub use document::{Document, Error};
pub use leaf::Leaf;
pub use node::{Branch, Node};
pub use pool::Pool;
pub use tree::Tree;

mod document;

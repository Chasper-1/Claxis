//! Rope-буфер: текст как дерево листьев над общим буфером.

use crate::chunk::chunk_size;
use crate::tree::Tree;

/// Причина отказа: диапазон не влезает в документ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Диапазон выходит за конец документа.
    OutOfRange {
        /// Начало диапазона.
        start: u32,
        /// Длина диапазона.
        len: u32,
        /// Длина документа.
        doc: u32,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::OutOfRange { start, len, doc } => {
                write!(
                    f,
                    "rope: range {start}..{} is outside {doc} bytes",
                    *start + *len
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// Rope-буфер: общий буфер и дерево листьев над ним.
#[derive(Debug)]
pub struct Document {
    buffer: Vec<u8>,
    tree: Tree,
}

impl Document {
    /// Документ из готовых байт.
    pub fn new(text: impl Into<Vec<u8>>) -> Self {
        let buffer = text.into();
        let tree = Tree::new(buffer.len() as u32);
        Self { buffer, tree }
    }

    /// Число байт в документе.
    pub fn len(&self) -> u32 {
        self.buffer.len() as u32
    }

    /// Документ пуст, если в нём нет ни байта.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Размер чанка для текущего объёма.
    pub fn chunk_size(&self) -> usize {
        chunk_size(self.buffer.len())
    }

    /// Дерево листьев над буфером.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Байты диапазона `start..start+len`.
    pub fn get(&self, start: u32, len: u32) -> Result<&[u8], Error> {
        self.range(start, len)?;
        Ok(&self.buffer[start as usize..(start + len) as usize])
    }

    /// Вставляет `data` в позицию `at`.
    pub fn insert(&mut self, at: u32, data: &[u8]) -> Result<(), Error> {
        let doc = self.len();
        if at > doc {
            return Err(Error::OutOfRange {
                start: at,
                len: data.len() as u32,
                doc,
            });
        }
        if data.is_empty() {
            return Ok(());
        }
        self.buffer
            .splice(at as usize..at as usize, data.iter().copied());
        self.tree.insert(at, data.len() as u32);
        Ok(())
    }

    /// Вырезает `len` байт начиная с позиции `at`.
    pub fn delete(&mut self, at: u32, len: u32) -> Result<(), Error> {
        self.range(at, len)?;
        if len == 0 {
            return Ok(());
        }
        self.buffer.drain(at as usize..(at + len) as usize);
        self.tree.delete(at, len);
        Ok(())
    }

    /// Меняет `len` байт начиная с позиции `at` на `data`.
    pub fn replace(&mut self, at: u32, len: u32, data: &[u8]) -> Result<(), Error> {
        self.range(at, len)?;
        self.delete(at, len)?;
        self.insert(at, data)
    }

    /// Копия всех байт документа.
    pub fn to_vec(&self) -> Vec<u8> {
        self.buffer.clone()
    }

    /// Диапазон обязан лежать в документе.
    fn range(&self, start: u32, len: u32) -> Result<(), Error> {
        let doc = self.len();
        if start.checked_add(len).is_none_or(|end| end > doc) {
            return Err(Error::OutOfRange { start, len, doc });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leaf::{Leaf, MAX_CHUNKS};
    use crate::node::Node;

    #[test]
    fn a_new_document_keeps_its_bytes() {
        let doc = Document::new("начало".as_bytes());
        assert_eq!(doc.len(), "начало".len() as u32);
        assert_eq!(doc.get(0, doc.len()).unwrap(), "начало".as_bytes());
    }

    #[test]
    fn insertion_keeps_the_buffer_whole() {
        let mut doc = Document::new("начало".as_bytes());
        doc.insert(0, "М ".as_bytes()).unwrap();
        doc.insert(doc.len(), " конец".as_bytes()).unwrap();
        assert_eq!(doc.to_vec(), "М начало конец".as_bytes());
        assert_eq!(doc.tree().len(), doc.len());
    }

    #[test]
    fn deletion_keeps_the_buffer_whole() {
        let mut doc = Document::new(b"abcdef".to_vec());
        doc.delete(1, 3).unwrap();
        assert_eq!(doc.to_vec(), b"aef");
        assert_eq!(doc.tree().len(), 3);
    }

    #[test]
    /// Шесть байт «при» срезаются, «пока» встают на их место.
    fn replace_is_delete_and_insert() {
        let mut doc = Document::new("привет".as_bytes().to_vec());
        doc.replace(0, 6, "пока".as_bytes()).unwrap();
        assert_eq!(doc.to_vec(), "покавет".as_bytes());
    }

    #[test]
    fn a_range_past_the_end_is_an_error() {
        let mut doc = Document::new("abc".as_bytes().to_vec());
        assert_eq!(
            doc.insert(4, b"d"),
            Err(Error::OutOfRange {
                start: 4,
                len: 1,
                doc: 3
            })
        );
        assert!(doc.delete(1, 3).is_err());
        assert!(doc.replace(0, usize::MAX as u32, b"x").is_err());
    }

    /// Листья слева направо.
    fn leaves(node: &Node, out: &mut Vec<Leaf>) {
        match node {
            Node::Leaf(leaf) => out.push(*leaf),
            Node::Branch(branch) => {
                for child in branch.children() {
                    leaves(child, out);
                }
            }
        }
    }

    /// Листья идут подряд, с нуля до конца документа, и ни один не тянет больше
    /// двух чанков.
    fn assert_tree_is_sound(doc: &Document) {
        let mut found = Vec::new();
        leaves(doc.tree().root(), &mut found);
        assert_eq!(found.first().map(Leaf::start), Some(0));
        assert_eq!(found.last().map(Leaf::end), Some(doc.len()));
        for (index, leaf) in found.iter().enumerate() {
            assert!(!leaf.is_empty(), "пустой лист {index} в дереве");
            assert!(
                leaf.chunks(doc.chunk_size()) <= MAX_CHUNKS,
                "лист {leaf:?} тянет больше двух чанков"
            );
            if index + 1 < found.len() {
                assert_eq!(
                    leaf.end(),
                    found[index + 1].start(),
                    "разрыв между листьями"
                );
            }
        }
        let total: u32 = found.iter().map(Leaf::len).sum();
        assert_eq!(total, doc.len(), "вес листьев разошёлся с документом");
    }

    /// Детерминированный генератор: тест должен падать всегда одинаково.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }

        fn below(&mut self, bound: u32) -> u32 {
            self.next() % bound
        }
    }

    #[test]
    /// Случайные правки: rope обязан совпадать с обычным вектором байт, а
    /// листья — оставаться подряд идущими кусками буфера.
    fn random_edits_match_a_plain_buffer() {
        let mut rng = Rng(42);
        let mut doc = Document::new(Vec::new());
        let mut plain = Vec::new();
        for step in 0..3000 {
            let len = plain.len() as u32;
            match rng.below(3) {
                0 => {
                    let at = rng.below(len + 1);
                    let take = rng.below(8) + 1;
                    let data: Vec<u8> = (0..take).map(|_| rng.next() as u8).collect();
                    doc.insert(at, &data).unwrap();
                    plain.splice(at as usize..at as usize, data);
                }
                1 => {
                    let len = plain.len() as u32;
                    if len == 0 {
                        continue;
                    }
                    let at = rng.below(len);
                    let take = (rng.below(len - at) + 1).min(len - at);
                    doc.delete(at, take).unwrap();
                    plain.drain(at as usize..(at + take) as usize);
                }
                _ => {
                    let len = plain.len() as u32;
                    if len == 0 {
                        continue;
                    }
                    let at = rng.below(len);
                    let take = (rng.below(len - at) + 1).min(len - at);
                    let put: Vec<u8> = (0..rng.below(6)).map(|_| rng.next() as u8).collect();
                    doc.replace(at, take, &put).unwrap();
                    plain.splice(at as usize..(at + take) as usize, put);
                }
            }
            assert_eq!(doc.len(), plain.len() as u32, "шаг {step}: разошлись длины");
            if step % 50 == 0 {
                assert_eq!(doc.to_vec(), plain, "шаг {step}: разошлись байты");
                assert_tree_is_sound(&doc);
            }
        }
        assert_eq!(doc.to_vec(), plain);
        assert_tree_is_sound(&doc);
    }

    #[test]
    /// Большой файл: чанк вырастает, листья всё равно не шире двух чанков.
    fn a_large_file_keeps_leaves_within_two_chunks() {
        let bytes = 512 * 1024u32;
        let mut doc = Document::new(vec![b'x'; bytes as usize]);
        assert_eq!(doc.chunk_size(), crate::chunk::MEDIUM);
        assert_tree_is_sound(&doc);
        for step in 0..20u32 {
            doc.insert(step * bytes / 20, "й".as_bytes()).unwrap();
            doc.delete(step * bytes / 20, 2).unwrap();
        }
        assert_eq!(doc.len(), bytes);
        assert_tree_is_sound(&doc);
    }

    #[test]
    fn many_edits_keep_the_buffer_and_the_tree_in_step() {
        let mut doc = Document::new(Vec::new());
        for index in 0..2000u32 {
            doc.insert(doc.len(), &[b'a' + (index % 26) as u8]).unwrap();
        }
        assert_eq!(doc.len(), 2000);
        for _ in 0..1000 {
            doc.delete(0, 1).unwrap();
        }
        assert_eq!(doc.len(), 1000);
        assert_eq!(doc.tree().len(), doc.len());
        assert!(
            doc.get(0, doc.len())
                .unwrap()
                .iter()
                .all(u8::is_ascii_lowercase)
        );
    }
}

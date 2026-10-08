use std::fmt;

use crate::arena::{Kind, Record, RecordId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Insert { pos: u32, add: RecordId },
    Delete { pos: u32, len: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    OutOfBounds {
        pos: u32,
        len: u32,
        doc_len: u32,
    },
    /// Данные крупнее арены: запись не помещается даже в пустую арену.
    ArenaFull,
}

impl Edit {
    /// Правка по записи арены: то, что вернёт отмена.
    pub fn from_record(record: &Record, id: RecordId) -> Self {
        match record.kind() {
            Kind::Insert => Edit::Insert {
                pos: record.pos(),
                add: id,
            },
            Kind::Delete => Edit::Delete {
                pos: record.pos(),
                len: record.len(),
            },
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfBounds { pos, len, doc_len } => {
                let end = pos.saturating_add(*len);
                write!(
                    f,
                    "range {pos}..{end} is outside the document of length {doc_len}"
                )
            }
            Error::ArenaFull => write!(f, "add arena is full"),
        }
    }
}

impl std::error::Error for Error {}

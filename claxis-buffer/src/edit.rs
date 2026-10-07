use std::fmt;

use crate::arena::AddId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Insert { pos: u32, add: AddId },
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

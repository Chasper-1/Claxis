use std::fmt;

use crate::arena::AddId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edit {
    Insert { pos: usize, add: AddId },
    Delete { pos: usize, len: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    OutOfBounds {
        pos: usize,
        len: usize,
        doc_len: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfBounds {
                pos,
                len,
                doc_len,
            } => {
                let end = pos.saturating_add(*len);
                write!(
                    f,
                    "range {pos}..{end} is outside the document of length {doc_len}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

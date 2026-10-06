use crate::arena::AddId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Original,
    Add(AddId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    pub source: Source,
    pub offset: usize,
    pub len: usize,
}

impl Segment {
    pub fn new(source: Source, offset: usize, len: usize) -> Self {
        Self {
            source,
            offset,
            len,
        }
    }

    pub fn end(&self) -> usize {
        self.offset + self.len
    }
}

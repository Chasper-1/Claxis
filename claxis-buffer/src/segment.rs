use crate::arena::AddId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Original,
    Add(AddId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    pub source: Source,
    pub offset: u32,
    pub len: u32,
}

impl Segment {
    pub fn new(source: Source, offset: u32, len: u32) -> Self {
        Self {
            source,
            offset,
            len,
        }
    }

    pub fn end(&self) -> u32 {
        self.offset + self.len
    }
}

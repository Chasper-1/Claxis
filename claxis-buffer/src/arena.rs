use std::ops::Index;

pub type AddId = usize;

pub const ARENA_CAPACITY: usize = 128 * 1024;

#[derive(Debug)]
pub struct AddArena {
    data: Vec<u8>,
    bump: usize,
    entries: Vec<(usize, usize)>,
}

impl Default for AddArena {
    fn default() -> Self {
        Self {
            data: vec![0; ARENA_CAPACITY],
            bump: 0,
            entries: Vec::new(),
        }
    }
}

impl AddArena {
    pub fn push(&mut self, data: &[u8]) -> Option<AddId> {
        if self.bump + data.len() > ARENA_CAPACITY {
            return None;
        }
        let start = self.bump;
        self.data[start..start + data.len()].copy_from_slice(data);
        self.bump += data.len();
        let id = self.entries.len();
        self.entries.push((start, data.len()));
        Some(id)
    }

    pub fn get(&self, add: AddId) -> Option<&[u8]> {
        let &(start, len) = self.entries.get(add)?;
        Some(&self.data[start..start + len])
    }

    pub fn start(&self, add: AddId) -> Option<usize> {
        self.entries.get(add).map(|&(start, _)| start)
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn remaining(&self) -> usize {
        ARENA_CAPACITY - self.bump
    }

    pub fn reset(&mut self) {
        self.bump = 0;
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Index<AddId> for AddArena {
    type Output = [u8];

    fn index(&self, add: AddId) -> &Self::Output {
        let (start, len) = self.entries[add];
        &self.data[start..start + len]
    }
}

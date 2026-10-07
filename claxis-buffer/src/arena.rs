use std::ops::Index;

pub type AddId = u32;

pub const ARENA_CAPACITY: usize = 1024 * 1024;

#[derive(Debug)]
pub struct AddArena {
    data: Vec<u8>,
    bump: usize,
    entries: Vec<(u32, u32)>,
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
        if data.len() > ARENA_CAPACITY {
            return None;
        }
        if self.bump + data.len() > ARENA_CAPACITY {
            return None;
        }
        let start = self.bump;
        self.data[start..start + data.len()].copy_from_slice(data);
        self.bump += data.len();
        let id = self.entries.len() as AddId;
        self.entries.push((start as u32, data.len() as u32));
        Some(id)
    }

    pub fn get(&self, add: AddId) -> Option<&[u8]> {
        let &(start, len) = self.entries.get(add as usize)?;
        Some(&self.data[start as usize..start as usize + len as usize])
    }

    pub fn start(&self, add: AddId) -> Option<u32> {
        self.entries.get(add as usize).map(|&(start, _)| start)
    }

    /// Диапазон записи в арене как смещение и длина.
    pub fn range(&self, add: AddId) -> Option<(u32, u32)> {
        self.entries.get(add as usize).copied()
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    pub fn remaining(&self) -> u32 {
        (ARENA_CAPACITY - self.bump) as u32
    }

    /// Переиспользование арены: указатель отматывается, записи чистятся.
    /// Данные не выделяются заново — bump-аллокация просто начинает с нуля.
    pub fn reset(&mut self) {
        self.bump = 0;
        self.entries.clear();
    }

    pub fn len(&self) -> u32 {
        self.entries.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Index<AddId> for AddArena {
    type Output = [u8];

    fn index(&self, add: AddId) -> &Self::Output {
        let (start, len) = self.entries[add as usize];
        &self.data[start as usize..start as usize + len as usize]
    }
}
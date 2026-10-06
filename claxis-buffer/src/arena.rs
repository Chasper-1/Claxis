use std::ops::Index;

pub type AddId = usize;

#[derive(Debug, Default)]
pub struct AddArena {
    entries: Vec<Vec<u8>>,
}

impl AddArena {
    pub fn push(&mut self, data: &[u8]) -> AddId {
        let id = self.entries.len();
        self.entries.push(data.to_vec());
        id
    }

    pub fn get(&self, add: AddId) -> Option<&[u8]> {
        self.entries.get(add).map(Vec::as_slice)
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
        &self.entries[add]
    }
}

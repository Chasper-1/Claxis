use crate::arena::{AddArena, AddId};
use crate::current::Current;
use crate::edit::{Edit, Error};
use crate::history::{History, Record};
use crate::segment::Segment;

pub struct Buffer {
    original: Vec<u8>,
    arena: AddArena,
    history: History,
    current: Current,
}

impl Buffer {
    pub fn new(original: impl AsRef<[u8]>) -> Self {
        let original = original.as_ref().to_vec();
        let current = Current::from_original(&original);
        Self {
            original,
            arena: AddArena::default(),
            history: History::default(),
            current,
        }
    }

    pub fn original(&self) -> &[u8] {
        &self.original
    }

    pub fn arena_entry(&self, add: AddId) -> Option<&[u8]> {
        self.arena.get(add)
    }

    pub fn add_count(&self) -> usize {
        self.arena.len()
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        self.current.segments()
    }

    pub fn len(&self) -> usize {
        self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }

    pub fn undo_stack(&self) -> &[Record] {
        self.history.undo_stack()
    }

    pub fn redo_stack(&self) -> &[Record] {
        self.history.redo_stack()
    }

    pub fn insert(&mut self, pos: usize, data: &[u8]) -> Result<(), Error> {
        self.check(pos, 0)?;
        if data.is_empty() {
            return Ok(());
        }
        let add = self.arena.push(data);
        let surgery = self.current.apply_insert(pos, add, data.len());
        self.history.record(Record {
            edit: Edit::Insert { pos, add },
            surgery,
        });
        Ok(())
    }

    pub fn delete(&mut self, pos: usize, len: usize) -> Result<(), Error> {
        self.check(pos, len)?;
        if len == 0 {
            return Ok(());
        }
        let surgery = self.current.apply_delete(pos, len);
        self.history.record(Record {
            edit: Edit::Delete { pos, len },
            surgery,
        });
        Ok(())
    }

    pub fn replace(&mut self, pos: usize, len: usize, data: &[u8]) -> Result<(), Error> {
        self.check(pos, len)?;
        self.delete(pos, len)?;
        self.insert(pos, data)
    }

    pub fn undo(&mut self) -> Option<Edit> {
        let (edit, surgery) = {
            let record = self.history.peek_undo()?;
            (record.edit, record.surgery)
        };
        self.current.undo_surgery(surgery);
        self.history.commit_undo();
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let (edit, surgery) = {
            let record = self.history.peek_redo()?;
            (record.edit, record.surgery)
        };
        self.current.apply_surgery(surgery);
        self.history.commit_redo();
        Some(edit)
    }

    pub fn rebuild(&mut self) {
        let active = self.history.active();
        self.current.rebuild(active);
    }

    pub fn read(&self) -> Vec<u8> {
        self.current.read(&self.original, &self.arena)
    }

    pub fn read_into(&self, out: &mut Vec<u8>) {
        self.current.read_into(out, &self.original, &self.arena);
    }

    #[cfg(test)]
    pub(crate) fn node_count(&self) -> usize {
        self.current.node_count()
    }

    fn check(&self, pos: usize, len: usize) -> Result<(), Error> {
        match pos.checked_add(len) {
            Some(end) if end <= self.len() => Ok(()),
            _ => Err(Error::OutOfBounds {
                pos,
                len,
                doc_len: self.len(),
            }),
        }
    }
}

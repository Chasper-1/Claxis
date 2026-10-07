use crate::arena::{AddArena, AddId};
use crate::current::Current;
use crate::edit::{Edit, Error};
use crate::history::{History, Record};
use crate::segment::Segment;

/// Байты снапшота: текущее состояние документа перед переездом `Original`.
pub struct Snapshot {
    original: Vec<u8>,
}

impl Snapshot {
    pub fn original(&self) -> &[u8] {
        &self.original
    }
}

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

    pub fn add_count(&self) -> u32 {
        self.arena.len()
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        self.current.segments(&self.arena)
    }

    pub fn len(&self) -> u32 {
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

    pub fn insert(&mut self, pos: u32, data: &[u8]) -> Result<(), Error> {
        self.check(pos, 0)?;
        if data.is_empty() {
            return Ok(());
        }
        let add = self.store(data)?;
        let off = self
            .arena
            .start(add)
            .expect("add только что записан в арену");
        let surgery = self.current.apply_insert(pos, add, off, data.len() as u32);
        self.history.record(Record {
            edit: Edit::Insert { pos, add },
            surgery,
        });
        Ok(())
    }

    pub fn delete(&mut self, pos: u32, len: u32) -> Result<(), Error> {
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

    pub fn replace(&mut self, pos: u32, len: u32, data: &[u8]) -> Result<(), Error> {
        self.check(pos, len)?;
        if !data.is_empty() && data.len() as u32 > self.arena.remaining() {
            self.snapshot_cycle();
        }
        self.delete(pos, len)?;
        self.insert(pos, data)
    }

    /// Снапшот: `Current` собирается в текст, текст становится `Original` #2,
    /// арена переиспользуется.
    pub fn snapshot(&mut self) -> Snapshot {
        let old = self.original.clone();
        self.snapshot_cycle();
        Snapshot { original: old }
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
    pub(crate) fn node_count(&self) -> u32 {
        self.current.node_count()
    }

    /// Записать данные в арену. Если остатка не хватает — цикл арены: снапшот
    /// переносит текущий текст в новый `Original`, арена освобождается.
    fn store(&mut self, data: &[u8]) -> Result<AddId, Error> {
        if data.len() > ARENA_CAPACITY_LIMIT {
            return Err(Error::ArenaFull);
        }
        if let Some(add) = self.arena.push(data) {
            return Ok(add);
        }
        self.snapshot_cycle();
        self.arena.push(data).ok_or(Error::ArenaFull)
    }

    fn snapshot_cycle(&mut self) {
        let text = self.read();
        self.original = text;
        self.arena.reset();
        self.current = Current::from_original(&self.original);
        // TODO: переезд истории на новый `Original` — см. отчёт.
        self.history = History::default();
    }

    fn check(&self, pos: u32, len: u32) -> Result<(), Error> {
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

/// Данные крупнее арены не помещаются ни в каком случае.
const ARENA_CAPACITY_LIMIT: usize = crate::arena::ARENA_CAPACITY;
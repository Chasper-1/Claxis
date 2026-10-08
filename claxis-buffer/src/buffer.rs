use crate::arena::{self, AddArena, Kind, RecordId};
use crate::current::Current;
use crate::edit::{Edit, Error};
use crate::history::History;
use crate::segment::Segment;

pub struct Buffer {
    original: Vec<u8>,
    /// Растущий буфер всего вставленного текста. Арена метаданных сюда не
    /// пишет: текст масштабируется свободно, ограничены только записи.
    added: Vec<u8>,
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
            added: Vec::new(),
            arena: AddArena::default(),
            history: History::default(),
            current,
        }
    }

    pub fn original(&self) -> &[u8] {
        &self.original
    }

    pub fn arena_record(&self, id: RecordId) -> Option<&arena::Record> {
        self.arena.get(id)
    }

    /// Участок растущего буфера вставленного текста.
    pub fn added(&self, off: u32, len: u32) -> Option<&[u8]> {
        self.added.get(off as usize..(off + len) as usize)
    }

    pub fn added_len(&self) -> u32 {
        self.added.len() as u32
    }

    pub fn add_count(&self) -> u32 {
        self.arena.len()
    }

    #[cfg(test)]
    pub(crate) fn parked_count(&self) -> usize {
        self.current.parked_count()
    }

    pub fn segments(&self) -> &[Segment] {
        self.current.segments()
    }

    pub fn len(&self) -> u32 {
        self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }

    pub fn undo_stack(&self) -> &[RecordId] {
        self.history.undo_stack()
    }

    pub fn redo_stack(&self) -> &[RecordId] {
        self.history.redo_stack()
    }

    /// Правки в порядке отмены: вершина undo — последняя.
    pub fn undo_edits(&self) -> Vec<Edit> {
        self.history
            .undo_stack()
            .iter()
            .filter_map(|id| {
                self.arena
                    .get(*id)
                    .map(|record| Edit::from_record(record, *id))
            })
            .collect()
    }

    pub fn redo_edits(&self) -> Vec<Edit> {
        self.history
            .redo_stack()
            .iter()
            .filter_map(|id| {
                self.arena
                    .get(*id)
                    .map(|record| Edit::from_record(record, *id))
            })
            .collect()
    }

    pub fn insert(&mut self, pos: u32, data: &[u8]) -> Result<(), Error> {
        self.check(pos, 0)?;
        if data.is_empty() {
            return Ok(());
        }
        if self.arena.remaining() == 0 {
            self.snapshot();
        }
        self.history.discard_redo();
        let text_off = self.push_text(data);
        let len = data.len() as u32;
        let id = self
            .arena
            .push(arena::Record::insert(pos, len, text_off))
            .ok_or(Error::ArenaFull)?;
        self.current.apply_insert(pos, text_off, len);
        self.history.push(id);
        Ok(())
    }

    pub fn delete(&mut self, pos: u32, len: u32) -> Result<(), Error> {
        self.check(pos, len)?;
        if len == 0 {
            return Ok(());
        }
        if self.arena.remaining() == 0 {
            self.snapshot();
        }
        self.history.discard_redo();
        // Удаление сначала применяется к документу: отложенные сегменты
        // известны только после него, а в запись они пишутся сразу.
        let parked = self.current.apply_delete(pos, len);
        let id = self
            .arena
            .push(arena::Record::delete(pos, len, parked))
            .ok_or(Error::ArenaFull)?;
        self.history.push(id);
        Ok(())
    }

    pub fn replace(&mut self, pos: u32, len: u32, data: &[u8]) -> Result<(), Error> {
        self.check(pos, len)?;
        if !data.is_empty() && data.len() as u32 > self.arena.remaining() {
            self.snapshot();
        }
        self.delete(pos, len)?;
        self.insert(pos, data)
    }

    /// Снапшот: документ собирается в текст, текст становится новым исходным,
    /// арена переиспользуется.
    pub fn snapshot(&mut self) {
        let text = self.read();
        self.original = text;
        self.added.clear();
        self.arena.reset();
        self.current = Current::from_original(&self.original);
        // TODO: переезд истории на новый исходный текст — см. отчёт.
        self.history = History::default();
    }

    pub fn undo(&mut self) -> Option<Edit> {
        let id = self.history.undo()?;
        let record = *self.arena.get(id)?;
        let edit = Edit::from_record(&record, id);
        match record.kind() {
            // Вставка отменяется удалением: сегменты уходят из чтения.
            Kind::Insert => {
                self.current.apply_delete(record.pos(), record.len());
            }
            // Удаление отменяется возвратом отложенных сегментов в чтение.
            Kind::Delete => {
                self.current
                    .restore_deleted(record.pos(), record.data(), record.len());
            }
        }
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let id = self.history.redo()?;
        let record = *self.arena.get(id)?;
        let edit = Edit::from_record(&record, id);
        match record.kind() {
            Kind::Insert => self
                .current
                .apply_insert(record.pos(), record.data(), record.len()),
            Kind::Delete => {
                self.current.redo_delete(record.pos(), record.len());
            }
        }
        Some(edit)
    }

    /// Пересборка из исходного текста и активной истории, по порядку.
    pub fn rebuild(&mut self) {
        // Поля берутся раздельно: записи отдаются по одному, без сбора в
        // промежуточный вектор — пересборка не выделяет память.
        let Buffer {
            arena,
            history,
            current,
            ..
        } = self;
        current.rebuild(
            history
                .undo_stack()
                .iter()
                .filter_map(|id| arena.get(*id))
                .map(|r| (r.kind(), r.pos(), r.len(), r.data())),
        );
    }

    pub fn read(&self) -> Vec<u8> {
        self.current.read(&self.original, &self.added)
    }

    pub fn read_into(&self, out: &mut Vec<u8>) {
        self.current.read_into(out, &self.original, &self.added);
    }

    /// Текст вставки уходит в растущий буфер — он ничем не ограничен.
    fn push_text(&mut self, data: &[u8]) -> u32 {
        let text_off = self.added.len() as u32;
        self.added.extend_from_slice(data);
        text_off
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

use std::fmt;
use std::mem::size_of;

use crate::arena::{Arena, DEFAULT_DEPTH, MAX_DEPTH, Record, RecordId};
use crate::current::Current;

/// Снимок памяти буфера — для диагностики и тестов.
#[derive(Clone, Copy, Debug)]
pub struct TreeMemory {
    pub nodes: usize,
    pub node_capacity: usize,
    pub free: usize,
    pub free_capacity: usize,
    pub leaves: usize,
    pub node_size: usize,
    pub added_len: usize,
    pub added_capacity: usize,
    pub original_capacity: usize,
    pub record_size: usize,
    pub arena_blocks: usize,
    pub arena_capacity: usize,
    pub pool_capacity: usize,
}

impl TreeMemory {
    /// Байт в пуле узлов. Ёмкость фиксирована и не меняется.
    pub fn tree_bytes(&self) -> usize {
        self.pool_capacity * self.node_size
    }
}

impl fmt::Display for TreeMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "узлов занято {}/{} (свободно {}), листьев {}, Added {} из {} байт",
            self.nodes,
            self.pool_capacity,
            self.free,
            self.leaves,
            self.added_len,
            self.added_capacity
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    OutOfBounds {
        anchor: u32,
        len: u32,
        doc_len: u32,
    },
    /// Глубина истории вне диапазона `1..=MAX_DEPTH`.
    ///
    /// Ноль бессмыслен: ни одна правка не поместилась бы, история обнулялась
    /// бы на каждом шаге. Больше `MAX_DEPTH` тоже незачем — всё, что не
    /// помещается, уходит на диск снапшотами.
    InvalidHistoryDepth {
        depth: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfBounds {
                anchor,
                len,
                doc_len,
            } => {
                write!(
                    f,
                    "range {anchor}..{} is outside the document of length {doc_len}",
                    anchor.saturating_add(*len)
                )
            }
            Error::InvalidHistoryDepth { depth } => write!(
                f,
                "глубина истории {depth} недопустима: допустимо от 1 до {MAX_DEPTH} записей"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Буфер Claxis: `Original`, `Added`, арена с двумя стеками и `Current`.
#[derive(Debug)]
pub struct Buffer {
    original: Vec<u8>,
    added: Vec<u8>,
    arena: Arena,
    current: Current,
    /// Сколько раз брался снапшот. Растёт только на снапшоте.
    snapshots: u64,
}

impl Buffer {
    /// Буфер с глубиной истории по умолчанию — полный блок в 8192 записи.
    pub fn new(original: impl AsRef<[u8]>) -> Self {
        Self::with_history_depth(original, DEFAULT_DEPTH).expect("глубина по умолчанию корректна")
    }

    /// Буфер с заданной глубиной истории.
    ///
    /// Глубина — сколько записей правки живёт в памяти до снапшота. Это
    /// единственная настройка размера: отдельного выбора размера арены нет.
    /// Чем меньше глубина, тем меньше и пул узлов дерева.
    ///
    /// Ноль отвергается: история не пережила бы ни одной правки. Больше
    /// `MAX_DEPTH` тоже — держать столько записей незачем.
    pub fn with_history_depth(original: impl AsRef<[u8]>, depth: u32) -> Result<Self, Error> {
        if !(1..=MAX_DEPTH).contains(&depth) {
            return Err(Error::InvalidHistoryDepth { depth });
        }
        let original = original.as_ref().to_vec();
        let arena = Arena::new(depth);
        let current = Current::from_original(
            original.len() as u32,
            crate::current::pool_capacity_for(depth),
        );
        Ok(Self {
            original,
            added: Vec::new(),
            arena,
            current,
            snapshots: 0,
        })
    }

    /// Глубина истории в записях.
    pub fn history_depth(&self) -> u32 {
        self.arena.depth() as u32
    }

    pub fn original(&self) -> &[u8] {
        &self.original
    }

    pub fn len(&self) -> u32 {
        self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }

    /// Сегменты в порядке документа — для тестов и отладки.
    pub fn segments(&self) -> Vec<crate::segment::Segment> {
        let mut out = Vec::with_capacity(self.current.seg_count() as usize);
        self.current.segments(&mut out);
        out
    }

    /// Высота дерева сегментов — для тестов и диагностики.
    pub fn tree_height(&self) -> usize {
        self.current.height()
    }

    /// Размер узла дерева в байтах.
    pub fn node_size() -> usize {
        Current::node_size()
    }

    /// Размер сегмента в байтах.
    pub fn segment_size() -> usize {
        Current::segment_size()
    }

    /// Статистика памяти: узлы дерева, их ёмкость и свободные.
    pub fn tree_memory(&self) -> TreeMemory {
        TreeMemory {
            nodes: self.current.node_count(),
            node_capacity: self.current.node_capacity(),
            free: self.current.free_count(),
            free_capacity: self.current.free_count(),
            leaves: self.current.seg_count() as usize,
            node_size: Current::node_size(),
            added_len: self.added.len(),
            added_capacity: self.added.capacity(),
            original_capacity: self.original.capacity(),
            record_size: size_of::<Record>(),
            arena_blocks: self.arena.block_count(),
            arena_capacity: self.arena.capacity_records(),
            pool_capacity: self.current.node_capacity_fixed(),
        }
    }

    pub fn undo_stack(&self) -> &[RecordId] {
        self.arena.undo_stack()
    }

    pub fn redo_stack(&self) -> &[RecordId] {
        self.arena.redo_stack()
    }

    pub fn record(&self, id: RecordId) -> Option<&Record> {
        self.arena.get(id)
    }

    pub fn remaining_records(&self) -> usize {
        self.arena.remaining_records()
    }

    /// Вставка: текст дописывается в `Added`, запись — в `undo`, затем сегмент.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) -> Result<(), Error> {
        if anchor > self.len() {
            return Err(Error::OutOfBounds {
                anchor,
                len: 0,
                doc_len: self.len(),
            });
        }
        if data.is_empty() {
            return Ok(());
        }
        self.snapshot_if_needed();
        self.discard_redo_with_cleanup();
        let text = self.added.len() as u32;
        self.added.extend_from_slice(data);
        let len = data.len() as u32;
        let id = self.arena.push(Record::insert(anchor, len, text));
        self.current.apply_insert(anchor, text, len, id);
        Ok(())
    }

    /// Удаление: запись в `undo`, диапазон физически вырезается из `Current`.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<(), Error> {
        self.check(anchor, len)?;
        if len == 0 {
            return Ok(());
        }
        self.snapshot_if_needed();
        self.discard_redo_with_cleanup();
        let _ = self.arena.push(Record::delete(anchor, len));
        self.current.apply_delete(anchor, len);
        Ok(())
    }

    /// Снапшот, когда история исчерпала свою глубину.
    ///
    /// Листья дерева ничем не ограничены: сколько их набралось, столько и
    /// живёт. Ограничение одно — **число записей**. Арена решила, сколько их
    /// помещается, и когда место кончилось, дерево схлопывается в новый
    /// `Original`, арена и `Added` очищаются, история уходит на диск (§11).
    ///
    /// Так память ограничена сверху: записей не больше глубины, а листьев не
    /// больше `2 · глубина + 1`, и пул узлов выделен ровно под это.
    fn snapshot_if_needed(&mut self) {
        if self.arena.remaining_records() == 0 {
            self.snapshot();
        }
    }

    /// Замена — удаление и вставка, никакой отдельной модели.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<(), Error> {
        self.delete(anchor, len)?;
        self.insert(anchor, data)
    }

    /// Отмена: верхняя запись `undo` уходит в `redo`, `Current` пересобирается.
    pub fn undo(&mut self) -> Option<RecordId> {
        let id = self.arena.undo()?;
        self.rebuild();
        self.snapshot_if_needed();
        Some(id)
    }

    /// Повтор отменённой правки: запись возвращается из `redo` в `undo`.
    pub fn redo(&mut self) -> Option<RecordId> {
        let id = self.arena.redo()?;
        self.rebuild();
        self.snapshot_if_needed();
        Some(id)
    }

    /// Сколько раз буфер брал снапшот — для бенчей и диагностики.
    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots
    }

    /// Отладочная проверка дерева: (узлов, глубина, цикл).
    #[cfg(test)]
    pub fn debug_walk(&self) -> (usize, usize, Option<u32>) {
        self.current.debug_walk()
    }

    /// Снапшот: текст из `Current` становится новым `Original`, всё обнуляется.
    pub fn snapshot(&mut self) {
        let text = self.read();
        self.original = text;
        self.added.clear();
        self.arena.reset();
        self.current.reset_to_single_leaf(crate::segment::Segment {
            record: crate::arena::ORIGINAL_ID,
            pos: 0,
            len: self.original.len() as u32,
            off: 0,
        });
        self.snapshots += 1;
    }

    /// Пересборка `Current` линейным проходом по активным записям `undo`.
    fn rebuild(&mut self) {
        let records = self
            .arena
            .undo_stack()
            .iter()
            .filter_map(|&id| self.arena.get(id).map(|r| (id, *r)));
        self.current.rebuild(self.original.len() as u32, records);
    }

    /// Новая правка при непустом `redo`: стек отменённых записей очищается.
    ///
    /// Тексты отменённых вставок не удаляются — `Added` обрезается по хвосту,
    /// и следующая вставка перезаписывает это место. Отменялись самые поздние
    /// вставки, поэтому их тексты лежат в самом конце и хвост достаточно
    /// срезать: ни удаления, ни перемещения памяти.
    ///
    /// Слоты записей в пуле арены не переиспользуются: идентификатор записи
    /// стабилен, на него ссылаются сегменты.
    ///
    /// Стек `redo` очищается на месте: забирать его через `mem::take` значило
    /// бы угнать вектор в локальную переменную и уронить вместе с ней —
    /// аллокация и освобождение на каждой правке после отмены.
    fn discard_redo_with_cleanup(&mut self) {
        if self.arena.redo_stack().is_empty() {
            return;
        }
        let tail = self
            .arena
            .redo_stack()
            .iter()
            .filter_map(|&id| self.arena.get(id))
            .filter_map(Record::text)
            .min();
        self.arena.clear_redo();
        if let Some(off) = tail {
            self.added.truncate(off as usize);
        }
    }

    pub fn read(&self) -> Vec<u8> {
        self.current.read(&self.original, &self.added)
    }

    pub fn read_into(&self, out: &mut Vec<u8>) {
        self.current.read_into(out, &self.original, &self.added);
    }

    /// Частичное чтение диапазона документа.
    pub fn read_range(&self, start: u32, end: u32) -> Vec<u8> {
        self.current
            .read_range(start, end, &self.original, &self.added)
    }

    fn check(&self, anchor: u32, len: u32) -> Result<(), Error> {
        match anchor.checked_add(len) {
            Some(end) if end <= self.len() => Ok(()),
            _ => Err(Error::OutOfBounds {
                anchor,
                len,
                doc_len: self.len(),
            }),
        }
    }
}

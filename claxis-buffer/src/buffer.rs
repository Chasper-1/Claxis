use std::fmt;
use std::mem::size_of;

use crate::arena::{Arena, MAX_DEPTH, Record, RecordId};
use crate::current::Current;
use crate::snapshot::{NullSink, SnapshotSink};

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
    /// Пул узлов дерева исчерпан.
    ///
    /// Случиться не должен: пул рассчитан на предел листьев при выбранной
    /// глубине истории. Значит либо ошибка в расчёте, либо снапшот не был
    /// сделан вовремя.
    TreePoolExhausted {
        capacity: usize,
        depth: u32,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    ///
    /// Буфер отдаёт структуру и передаёт каталогу значения. Как они попадут в
    /// текст — дело каталога: в конфиге сообщение пишется целиком, с
    /// подстановками в тех местах, куда их поставил переводчик.
    pub fn message(&self, messages: &dyn crate::messages::Messages) -> String {
        match self {
            Error::OutOfBounds {
                anchor,
                len,
                doc_len,
            } => messages.out_of_bounds(*anchor, anchor.saturating_add(*len), *doc_len),
            Error::InvalidHistoryDepth { depth } => {
                messages.invalid_history_depth(*depth, 1, MAX_DEPTH)
            }
            Error::TreePoolExhausted { capacity, depth } => {
                messages.tree_pool_exhausted(*capacity, *depth)
            }
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
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
    /// Куда уходят снятые снапшоты. По умолчанию — никуда.
    sink: Box<dyn SnapshotSink>,
    /// Документ, которому принадлежит буфер. Нужен снапшоту.
    file: Option<std::path::PathBuf>,
    /// Сколько раз брался снапшот. Растёт только на снапшоте.
    snapshots: u64,
}

impl Buffer {
    /// Буфер с глубиной истории по умолчанию.
    ///
    /// Глубина берётся из настроек, а не из константы арены: когда появится
    /// конфиг, значение придёт оттуда, и менять тут ничего не придётся.
    /// Значение известно корректное, поэтому `Result` не нужен.
    pub fn new(original: impl AsRef<[u8]>) -> Self {
        Self::build(
            original.as_ref(),
            crate::defaults::buffer::HISTORY_DEPTH.value,
        )
        .expect("the default history depth is valid by construction")
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
        Self::build(original.as_ref(), depth)
    }

    /// Построить буфер с уже проверенной глубиной.
    fn build(original: &[u8], depth: u32) -> Result<Self, Error> {
        let original = original.to_vec();
        let arena = Arena::new(depth);
        let current = Current::from_original(
            original.len() as u32,
            crate::current::pool_capacity_for(depth),
            depth,
        )?;
        Ok(Self {
            original,
            added: Vec::new(),
            arena,
            current,
            sink: Box::new(NullSink),
            file: None,
            snapshots: 0,
        })
    }

    /// Привязать снапшоты к документу и приёмнику.
    ///
    /// Буфер сам про диск не знает: он отдаёт текст приёмнику, а что с ним
    /// делать — решает внешний слой. Без приёмника снапшот никуда не пишется
    /// и живёт только в памяти.
    pub fn set_sink(&mut self, file: impl Into<std::path::PathBuf>, sink: Box<dyn SnapshotSink>) {
        self.file = Some(file.into());
        self.sink = sink;
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
        self.snapshot_if_needed()?;
        self.discard_redo_with_cleanup();
        let text = self.added.len() as u32;
        self.added.extend_from_slice(data);
        let len = data.len() as u32;
        let id = self.arena.push(Record::insert(anchor, len, text));
        self.current.apply_insert(anchor, text, len, id)?;
        Ok(())
    }

    /// Удаление: запись в `undo`, диапазон физически вырезается из `Current`.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<(), Error> {
        self.check(anchor, len)?;
        if len == 0 {
            return Ok(());
        }
        self.snapshot_if_needed()?;
        self.discard_redo_with_cleanup();
        let _ = self.arena.push(Record::delete(anchor, len));
        self.current.apply_delete(anchor, len)?;
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
    fn snapshot_if_needed(&mut self) -> Result<(), Error> {
        if self.arena.remaining_records() == 0 {
            self.snapshot()?;
        }
        Ok(())
    }

    /// Замена — удаление и вставка, никакой отдельной модели.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<(), Error> {
        self.delete(anchor, len)?;
        self.insert(anchor, data)
    }

    /// Отмена: верхняя запись `undo` уходит в `redo`, `Current` пересобирается.
    pub fn undo(&mut self) -> Result<Option<RecordId>, Error> {
        let Some(id) = self.arena.undo() else {
            return Ok(None);
        };
        self.rebuild()?;
        self.snapshot_if_needed()?;
        Ok(Some(id))
    }

    /// Повтор отменённой правки: запись возвращается из `redo` в `undo`.
    pub fn redo(&mut self) -> Result<Option<RecordId>, Error> {
        let Some(id) = self.arena.redo() else {
            return Ok(None);
        };
        self.rebuild()?;
        self.snapshot_if_needed()?;
        Ok(Some(id))
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
    pub fn snapshot(&mut self) -> Result<(), Error> {
        let text = self.read();
        self.original = text;
        self.added.clear();
        self.arena.reset();
        self.current.reset_to_single_leaf(crate::segment::Segment {
            record: crate::arena::ORIGINAL_ID,
            pos: 0,
            len: self.original.len() as u32,
            off: 0,
        })?;
        self.snapshots += 1;
        // Отдать снапшот приёмнику: буфер про диск не знает и просто передаёт
        // текст. Ошибка сохранения не ломает правку — потеря кеша не должна
        // стоить пользователю работы.
        if let Some(file) = self.file.clone() {
            self.sink.accept(&file, &self.original);
        }
        Ok(())
    }

    /// Пересборка `Current` линейным проходом по активным записям `undo`.
    fn rebuild(&mut self) -> Result<(), Error> {
        let records = self
            .arena
            .undo_stack()
            .iter()
            .filter_map(|&id| self.arena.get(id).map(|r| (id, *r)));
        self.current.rebuild(self.original.len() as u32, records)?;
        Ok(())
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

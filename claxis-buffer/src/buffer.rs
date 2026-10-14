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

/// Записать запись правки в историю.
fn write_one(out: &mut crate::history::Bytes, record: &Record) {
    match record {
        Record::Insert { anchor, head, text } => {
            crate::history::write_record(out, crate::history::INSERT_TAG, *anchor, *head, *text)
        }
        Record::Delete { anchor, head, text } => {
            crate::history::write_record(out, crate::history::DELETE_TAG, *anchor, *head, *text)
        }
    }
}

/// Разобрать стек истории в записи правок.
///
/// Мусор на входе обрывается, а не ломает документ: неполная запись — это
/// потеря одной записи отмены, а не битый текст.
fn read_all(bytes: &[u8]) -> Vec<Record> {
    let len = crate::history::RECORD_LEN;
    let mut out = Vec::with_capacity(bytes.len() / len);
    for part in bytes.chunks_exact(len) {
        let Some((tag, anchor, head, text)) = crate::history::read_record(part) else {
            break;
        };
        out.push(match tag {
            crate::history::INSERT_TAG => Record::Insert { anchor, head, text },
            crate::history::DELETE_TAG => Record::Delete { anchor, head, text },
            _ => break,
        });
    }
    out
}

/// Буфер Claxis: `Original`, `Added`, арена с двумя стеками и `Current`.
#[derive(Debug)]
pub struct Buffer {
    original: Vec<u8>,
    // Удалённый текст тоже лежит в `added`: отменить удаление — значит
    // вставить обратно, а сегмент читает текст по одной ссылке. Второй буфер
    // означал бы вторую ветку кода и вторую границу памяти.
    added: Vec<u8>,
    /// Смещения текста, вставленного отменой удаления.
    ///
    /// Такой текст не принадлежит ни одной записи истории: запись удаления
    /// лежит в отброшенной ветке, и её обрезка унесла бы текст, на который
    /// уже ссылается живой сегмент. Смещения помнятся, чтобы хвост не
    /// дошёл до них.
    phantom: Vec<u32>,
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
    ///
    /// Паники здесь нет намеренно: глубина задана самим крейтом, но если она
    /// вдруг окажется негодной, редактор всё равно должен открыться. Тогда
    /// документ просто создаётся без исходного текста — пустой, но рабочий.
    pub fn new(original: impl AsRef<[u8]>) -> Self {
        let original = original.as_ref();
        let depth = crate::defaults::buffer::HISTORY_DEPTH.value;
        match Self::build(original, depth) {
            Ok(buffer) => buffer,
            Err(_) => Self {
                original: Vec::new(),
                added: Vec::new(),
                phantom: Vec::new(),
                arena: Arena::new(depth),
                current: Current::empty(depth),
                sink: Box::new(NullSink),
                file: None,
                snapshots: 0,
            },
        }
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
            phantom: Vec::new(),
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

    /// Записи активной ветки, от корня к текущей вершине.
    pub fn undo_stack(&self) -> Vec<RecordId> {
        self.arena.path()
    }

    /// Записи отброшенной ветки, до которых можно повторить.
    pub fn redo_stack(&self) -> Vec<RecordId> {
        self.arena.redo_path()
    }

    /// Отладочный снимок вершин истории.
    pub fn debug_nodes(&self) -> Vec<(Option<RecordId>, Option<RecordId>, bool)> {
        self.arena.debug_nodes()
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
        // Удалённый текст запоминается: без него удаление нечем отменить.
        let text = self.added.len() as u32;
        self.cut_into_added(anchor, len);
        self.arena.push(Record::delete(anchor, len, text));
        self.current.apply_delete(anchor, len)?;
        Ok(())
    }

    /// Вырезать диапазон документа в конец `added`.
    ///
    /// Куски берутся обходом дерева и склеиваются в порядке документа: вернуть
    /// их — значит вставить обратно ровно этот кусок, посимвольно.
    fn cut_into_added(&mut self, anchor: u32, len: u32) {
        let mut pieces = Vec::new();
        self.current.range_pieces(anchor, anchor + len, &mut pieces);
        // Копии собираются до записи: источник и приёмник — один и тот же
        // буфер, и писать в него на ходу обхода нельзя.
        let chunks: Vec<Vec<u8>> = pieces
            .iter()
            .map(|&(from_added, off, len)| {
                let source = if from_added {
                    &self.added
                } else {
                    &self.original
                };
                let at = off as usize;
                source[at..at + len as usize].to_vec()
            })
            .collect();
        for chunk in chunks {
            self.added.extend_from_slice(&chunk);
        }
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

    /// Отмена одной правки.
    ///
    /// Применяется обратная операция, а не пересборка дерева по всей истории:
    /// стоимость не зависит от того, сколько правок накопилось. Запись, на
    /// которой мы стоим, уходит к родителю.
    pub fn undo(&mut self) -> Result<Option<RecordId>, Error> {
        let Some(id) = self.arena.undo() else {
            return Ok(None);
        };
        let Some(record) = self.arena.get(id).copied() else {
            return Ok(None);
        };
        self.apply_inverse(record)?;
        Ok(Some(id))
    }

    /// Повтор отменённой правки: возвращаемся к ребёнку и применяем запись.
    pub fn redo(&mut self) -> Result<Option<RecordId>, Error> {
        let Some(id) = self.arena.redo() else {
            return Ok(None);
        };
        let Some(record) = self.arena.get(id).copied() else {
            return Ok(None);
        };
        self.apply_record(record, id)?;
        Ok(Some(id))
    }

    /// Применить запись как есть: повтор правки.
    fn apply_record(&mut self, record: Record, id: RecordId) -> Result<(), Error> {
        match record {
            Record::Insert { anchor, head, text } => {
                let len = head - anchor;
                self.current.apply_insert(anchor, text, len, id)
            }
            Record::Delete { anchor, head, .. } => self.current.apply_delete(anchor, head - anchor),
        }
    }

    /// Отменить запись обратной операцией.
    fn apply_inverse(&mut self, record: Record) -> Result<(), Error> {
        match record {
            // Отмена вставки — вырезать то, что она вставила.
            Record::Insert { anchor, head, .. } => self.current.apply_delete(anchor, head - anchor),
            // Отмена удаления — вернуть то, что было вырезано.
            Record::Delete { anchor, head, text } => {
                let len = head - anchor;
                // Текст копируется в конец `added`, а не берётся по старой
                // ссылке: он принадлежит записи удаления, а та лежит в
                // отброшенной ветке и её текст будет срезан при следующей
                // правке. Ссылка на срезанное место — это битый документ.
                let at = self.added.len() as u32;
                let piece: Vec<u8> = self
                    .added
                    .get(text as usize..text as usize + len as usize)
                    .unwrap_or_default()
                    .to_vec();
                self.added.extend_from_slice(&piece);
                // Запоминаем конец фантомного текста: хвост нельзя резать
                // ниже этой точки, иначе сегмент останется ссылаться на
                // срезанное место.
                self.phantom.push(at + len);
                // Запись в историю не нужна: эта правка лишь возвращает
                // документ к состоянию до удаления.
                self.current.apply_insert(anchor, at, len, u32::MAX)?;
                Ok(())
            }
        }
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
    ///
    /// История уходит вместе со снапшотом: текст без истории бесполезен —
    /// после перезапуска отменять было бы нечего. Поэтому она собирается
    /// **до** сброса арены и передаётся приёмнику вместе с текстом.
    pub fn snapshot(&mut self) -> Result<(), Error> {
        let text = self.read();
        // История собирается только когда есть куда её положить: сборка
        // копирует тексты вставок, а без приёмника они сразу никому не нужны.
        let history = self.file.as_ref().map(|_| self.collect_history());
        self.original = text;
        self.added.clear();
        self.phantom.clear();
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
        if let (Some(file), Some(history)) = (self.file.clone(), history) {
            self.sink.accept(&file, &self.original, &history);
        }
        Ok(())
    }

    /// История в виде байт: активная ветка, redo-ветка и тексты.
    ///
    /// Активная ветка идёт от корня к текущей вершине — в том же порядке, в
    /// каком их читает [`Buffer::restore_history`].
    fn collect_history(&self) -> crate::history::History {
        let mut undo = crate::history::Bytes::new();
        for &id in &self.arena.path() {
            if let Some(record) = self.arena.get(id) {
                write_one(&mut undo, record);
            }
        }
        let mut redo = crate::history::Bytes::new();
        for id in self.arena.redo_path() {
            if let Some(record) = self.arena.get(id) {
                write_one(&mut redo, record);
            }
        }
        crate::history::History::new(self.original.clone(), undo, redo, self.added.clone())
    }

    /// Вернуть историю, сохранённую снапшотом.
    ///
    /// Записи кладутся в арену как есть, а дерево строится заново по ним:
    /// иначе после перезапуска документ был бы верным, а отменять было бы
    /// нечего. Тексты вставок приходят из снапшота — здесь `Added` пуст, и он
    /// заполняется ими до пересборки дерева.
    pub fn restore_history(&mut self, history: &crate::history::History) -> Result<(), Error> {
        if history.is_empty() {
            return Ok(());
        }
        // База из истории становится новым `Original`: записи описывают
        // переход именно от него, и применять их к чему-то другому — значит
        // получить другой документ.
        self.original = history.base().to_vec();
        self.added.clear();
        self.added.extend_from_slice(history.added());
        let undo = read_all(history.undo());
        let redo = read_all(history.redo());
        self.arena.restore(undo, redo);
        // Дерево собирается заново по активным записям: `Original` уже
        // содержит текст снапшота, а записи объясняют, чем он стал.
        self.rebuild()
    }

    /// Пересборка `Current` линейным проходом по активным записям `undo`.
    fn rebuild(&mut self) -> Result<(), Error> {
        let path = self.arena.path();
        let records = path
            .iter()
            .filter_map(|&id| self.arena.get(id).map(|r| (id, *r)));
        self.current.rebuild(self.original.len() as u32, records)?;
        Ok(())
    }

    /// Новая правка отбрасывает redo-ветку.
    ///
    /// Тексты отменённых правок не удаляются — `added` обрезается по хвосту,
    /// и следующая правка перезаписывает это место. Отменялись самые поздние
    /// правки, поэтому их тексты лежат в самом конце и хвост достаточно
    /// срезать: ни удаления, ни перемещения памяти.
    ///
    /// Слоты вершин переиспользуются: отброшенная ветка больше не нужна, и её
    /// вершины уходят в свободный список.
    fn discard_redo_with_cleanup(&mut self) {
        // Ветка обрезается с хвоста текста: обход по детям возвращает
        // наименьшее смещение, с которого текст больше никому не нужен.
        let tail = self.arena.discard_child();
        // Текст, вставленный отменой удаления, резать нельзя: на него уже
        // ссылаются живые сегменты, а его запись лежит в отброшенной ветке.
        // Хвост уходит не ниже конца этого текста.
        let keep = self.phantom.iter().copied().max();
        self.phantom.clear();
        let cut = match (tail, keep) {
            (Some(tail), Some(keep)) => Some(tail.max(keep)),
            (tail, None) => tail,
            (None, _) => None,
        };
        if let Some(off) = cut {
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

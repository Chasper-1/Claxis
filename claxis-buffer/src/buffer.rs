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
                let start = off as usize;
                source[start..start + len as usize].to_vec()
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
                let appended = self.added.len() as u32;
                let piece: Vec<u8> = self
                    .added
                    .get(text as usize..text as usize + len as usize)
                    .unwrap_or_default()
                    .to_vec();
                self.added.extend_from_slice(&piece);
                // Запоминаем конец фантомного текста: хвост нельзя резать
                // ниже этой точки, иначе сегмент останется ссылаться на
                // срезанное место.
                self.phantom.push(appended + len);
                // Запись в историю не нужна: эта правка лишь возвращает
                // документ к состоянию до удаления.
                self.current.apply_insert(anchor, appended, len, u32::MAX)?;
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

#[cfg(test)]
mod tests {
    use crate::buffer::Buffer;

    fn b(text: &str) -> Buffer {
        Buffer::new(text.as_bytes())
    }

    #[test]
    fn insert_splits_original_and_reads_back() {
        let mut buf = b("abcdefghij");
        buf.insert(5, b"XY").unwrap();
        assert_eq!(buf.read(), b"abcdeXYfghij");
    }

    #[test]
    fn insert_at_end_appends() {
        let mut buf = b("abc");
        buf.insert(3, b"def").unwrap();
        assert_eq!(buf.read(), b"abcdef");
    }

    #[test]
    fn insert_at_zero_prepends() {
        let mut buf = b("abc");
        buf.insert(0, b"XY").unwrap();
        assert_eq!(buf.read(), b"XYabc");
    }

    #[test]
    fn delete_excludes_range_and_undo_restores_exactly() {
        let mut buf = b("The quick brown fox");
        buf.delete(4, 5).unwrap();
        assert_eq!(buf.read(), b"The  brown fox");
        buf.undo().unwrap().expect("удаление отменяемо");
        assert_eq!(buf.read(), b"The quick brown fox");
    }

    #[test]
    fn delete_across_segment_boundaries() {
        let mut buf = b("aaa");
        buf.insert(3, b"bbb").unwrap();
        buf.delete(1, 4).unwrap();
        assert_eq!(buf.read(), b"ab");
        buf.undo().unwrap().expect("отменяемо");
        assert_eq!(buf.read(), b"aaabbb");
    }

    #[test]
    fn delete_to_the_end_of_document() {
        let mut buf = b("abcdef");
        buf.delete(3, 3).unwrap();
        assert_eq!(buf.read(), b"abc");
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn delete_never_touches_deleted_again() {
        // Два удаления подряд не должны вырезать уже вырезанное.
        let mut buf = b("0123456789");
        buf.delete(2, 3).unwrap();
        assert_eq!(buf.read(), b"0156789");
        buf.delete(0, 2).unwrap();
        assert_eq!(buf.read(), b"56789");
        buf.undo().unwrap().expect("отменяемо");
        assert_eq!(buf.read(), b"0156789");
    }

    #[test]
    fn undo_insert_removes_text_and_redo_restores() {
        let mut buf = b("abc");
        buf.insert(3, b"XY").unwrap();
        buf.undo().unwrap().expect("есть что отменять");
        assert_eq!(buf.read(), b"abc");
        buf.redo().unwrap().expect("есть что повторять");
        assert_eq!(buf.read(), b"abcXY");
    }

    #[test]
    fn undo_redo_walk_history_linearly() {
        let mut buf = b("base");
        for i in 0..5 {
            buf.insert(i as u32, b"ab").unwrap();
        }
        let end = buf.read();
        for _ in 0..5 {
            buf.undo().unwrap().expect("отменяемо");
        }
        assert_eq!(buf.read(), b"base");
        assert!(buf.undo().unwrap().is_none());
        for _ in 0..5 {
            buf.redo().unwrap().expect("повторяемо");
        }
        assert_eq!(buf.read(), end);
        assert!(buf.redo().unwrap().is_none());
    }

    #[test]
    fn new_edit_after_undo_clears_redo() {
        let mut buf = b("");
        buf.insert(0, b"A").unwrap();
        buf.insert(1, b"B").unwrap();
        buf.undo().unwrap().unwrap();
        assert!(!buf.redo_stack().is_empty());

        buf.insert(0, b"C").unwrap();
        assert_eq!(buf.read(), b"CA", "вставка в начало, а не замена");
        assert!(buf.redo_stack().is_empty(), "ветка отброшена");
        assert!(buf.redo().unwrap().is_none());
    }

    #[test]
    fn replace_is_delete_then_insert() {
        // Замена в этом буфере — это удаление и вставка, то есть две записи
        // истории: одна отмена убирает только вставленное.
        let mut buf = b("The quick brown fox");
        buf.replace(4, 5, b"slow").unwrap();
        assert_eq!(buf.read(), b"The slow brown fox");
        assert_eq!(buf.undo_stack().len(), 2, "удаление и вставка");

        buf.undo().unwrap().expect("вставка отменяема");
        assert_eq!(buf.read(), b"The  brown fox", "вырезанное ещё на месте");
        buf.undo().unwrap().expect("удаление отменяемо");
        assert_eq!(buf.read(), b"The quick brown fox");
    }

    #[test]
    fn nested_inserts_resolve_to_flat_document() {
        // Несколько вставок встык: чтение обязано дать ровно то, что вставляли.
        let mut buf = b("");
        for i in 0..5u32 {
            buf.insert(i, format!("{i}").as_bytes()).unwrap();
        }
        assert_eq!(buf.read(), b"01234");
    }

    #[test]
    fn delete_undo_preserves_state_when_deleted_was_inserted_earlier() {
        // Удаляем кусок, который сами вставили: отмена обязана вернуть его
        // целиком, вместе с вставкой, под которой он лежал.
        let mut buf = b("abc");
        buf.insert(3, b"XYZ").unwrap();
        assert_eq!(buf.read(), b"abcXYZ");
        // Убираем `bcX` — четыре байта начиная с позиции 1.
        buf.delete(1, 4).unwrap();
        assert_eq!(buf.read(), b"aZ", "вырезано четыре байта: bcXY");
        buf.undo().unwrap().expect("отменяемо");
        assert_eq!(buf.read(), b"abcXYZ");
    }

    #[test]
    fn read_range_is_substring_of_full_read() {
        let buf = b("The quick brown fox");
        let full = buf.read();
        for anchor in 0..=full.len() {
            for end in anchor..=full.len() {
                assert_eq!(
                    buf.read_range(anchor as u32, end as u32),
                    full[anchor..end].to_vec(),
                    "диапазон {anchor}..{end}"
                );
            }
        }
    }

    #[test]
    fn read_range_after_deletes_and_inserts() {
        let mut buf = b("");
        for i in 0..10u32 {
            buf.insert(i, b"xy").unwrap();
        }
        buf.delete(4, 6).unwrap();
        let full = buf.read();
        assert_eq!(buf.read_range(0, buf.len()), full);
        assert_eq!(buf.read_range(2, 6), full[2..6].to_vec());
        assert_eq!(buf.read_range(3, 3), Vec::<u8>::new());
    }

    #[test]
    fn out_of_bounds_is_an_error() {
        let mut buf = b("abc");
        assert!(buf.delete(2, 5).is_err());
        assert!(buf.insert(99, b"x").is_err());
        assert_eq!(buf.read(), b"abc", "документ не тронут");
    }

    #[test]
    fn zero_length_ops_add_no_history() {
        let mut buf = b("abc");
        buf.insert(1, b"").unwrap();
        buf.delete(1, 0).unwrap();
        buf.replace(1, 0, b"").unwrap();
        assert!(
            buf.undo_stack().is_empty(),
            "пустые правки в историю не идут"
        );
        assert_eq!(buf.read(), b"abc");
    }

    #[test]
    fn undo_and_redo_are_lifo_stacks() {
        // Записи идут одна на одну, сверху последняя.
        let mut buf = b("");
        buf.insert(0, b"A").unwrap();
        buf.insert(1, b"B").unwrap();
        buf.insert(2, b"C").unwrap();
        assert_eq!(buf.undo_stack().len(), 3);

        // Путь идёт от корня, поэтому последняя запись — вершина, и отменяется
        // именно она.
        let id = *buf.undo_stack().last().unwrap();
        assert_eq!(buf.record(id).unwrap().anchor(), 2);
        assert_eq!(buf.record(id).unwrap().head(), 3);

        // Отмена снимает вершину, повтор возвращает её на место.
        assert_eq!(buf.undo().unwrap(), Some(id));
        assert_eq!(buf.undo_stack().len(), 2, "вершина ушла из пути");
        assert!(!buf.redo_stack().is_empty());
        assert_eq!(buf.redo().unwrap(), Some(id));
        assert_eq!(buf.undo_stack().len(), 3, "запись вернулась в путь");
    }

    #[test]
    fn segments_hold_record_and_text_refs() {
        let mut buf = b("abcde");
        buf.insert(2, b"XY").unwrap();
        let segs = buf.segments();
        assert_eq!(segs.len(), 3);
        // Исходный текст без записи.
        assert_eq!(segs[0].record, crate::arena::ORIGINAL_ID);
        assert_eq!(segs[0].len, 2);
        // Вставка — своя запись, текст из Added.
        let insert = segs[1];
        assert_ne!(insert.record, crate::arena::ORIGINAL_ID);
        assert_eq!(insert.len, 2);
        assert_eq!(segs[2].len, 3);
    }

    #[test]
    fn tree_stays_balanced_under_many_inserts() {
        // Дерево не должно вырождаться в список: высота обязана расти логарифмом.
        let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
        for i in 0..5_000u32 {
            buf.insert(i, b"x").unwrap();
        }
        let height = buf.tree_height();
        let leaves = buf.segments().len();
        assert!(
            leaves >= 5_000,
            "листьев должно быть не меньше 5000, got {leaves}"
        );
        // AVL-дерево из n листьев: высота <= 1.44*log2(n+2).
        let bound = 1.45 * (leaves as f64 + 2.0).log2();
        assert!(
            height as f64 <= bound,
            "высота {height} превышает границу {bound:.1} при {leaves} листьях"
        );
    }

    #[test]
    fn tree_stays_balanced_under_deletes() {
        let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
        for _ in 0..5_000 {
            buf.insert(buf.len(), b"xy").unwrap();
        }
        // Удаляем каждый второй байт с начала — листья тают, баланс должен
        // остаться.
        while !buf.is_empty() {
            buf.delete(0, 1).unwrap();
        }
        assert!(buf.is_empty());
        assert_eq!(buf.segments().len(), 0);
        assert_eq!(buf.tree_height(), 0);

        // Снова наполняем — высота обязана быть логарифмической.
        for _ in 0..5_000 {
            buf.insert(buf.len(), b"xy").unwrap();
        }
        let leaves = buf.segments().len();
        let height = buf.tree_height();
        let bound = 1.45 * (leaves as f64 + 2.0).log2();
        assert!(
            height as f64 <= bound,
            "высота {height} превышает границу {bound:.1} при {leaves} листьях"
        );
        assert_eq!(buf.read().len(), 10_000);
    }

    #[test]
    fn node_pool_does_not_grow_without_bound() {
        // Освобождённые узлы идут в пул повторного использования: после волны
        // вставок и удалений размер пула заметно меньше суммарного числа
        // операций.
        let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
        for _ in 0..4_000 {
            buf.insert(buf.len(), b"ab").unwrap();
        }
        assert_eq!(buf.read().len(), 8_000);
        let after_inserts = buf.segments().len();

        // Удаляем всё, потом наполняем заново — пул переиспользуется, листья
        // не растут бесконечно.
        while !buf.is_empty() {
            buf.delete(0, 1).unwrap();
        }
        assert!(buf.segments().is_empty());
        for _ in 0..4_000 {
            buf.insert(buf.len(), b"ab").unwrap();
        }
        assert_eq!(buf.read().len(), 8_000);
        assert!(
            buf.segments().len() <= after_inserts,
            "листьев снова выросло: {} против {after_inserts}",
            buf.segments().len()
        );
    }

    #[test]
    fn snapshot_moves_current_into_new_original() {
        let mut buf = b("hello");
        buf.insert(5, b" world").unwrap();
        buf.snapshot().unwrap();
        assert_eq!(buf.original(), b"hello world");
        assert_eq!(buf.read(), b"hello world");
        assert!(buf.undo_stack().is_empty());
        buf.insert(11, b"!").unwrap();
        assert_eq!(buf.read(), b"hello world!");
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"hello world");
    }

    #[test]
    fn snapshot_keeps_document_bounded_in_memory() {
        // Листья ничем не ограничены — ограничено число записей. Когда арена
        // исчерпала глубину, снапшот схлопывает дерево в новый Original. Текст
        // при этом не теряется.
        const DEPTH: u32 = 1_000;
        let mut buf = Buffer::with_history_depth(b"", DEPTH).unwrap();
        let mut peak_records = 0usize;
        for _ in 0..(DEPTH as usize * 3) {
            buf.insert(buf.len(), b"xy").unwrap();
            peak_records = peak_records.max(buf.undo_stack().len());
        }
        assert!(
            peak_records <= DEPTH as usize,
            "записей не должно стать больше глубины"
        );
        assert_eq!(
            peak_records,
            DEPTH as usize - 1,
            "снапшот берётся перед записью, поэтому в арене на одну запись меньше"
        );
        assert!(buf.snapshots_taken() > 0, "снапшот обязан был сработать");
        assert_eq!(buf.read().len(), DEPTH as usize * 6);

        // Правки продолжаются от нового исходного текста.
        buf.insert(buf.len(), b"!").unwrap();
        let text = buf.read();
        assert_eq!(text[text.len() - 1], b'!');
        assert_eq!(text.len(), DEPTH as usize * 6 + 1);
        assert!(
            text[..DEPTH as usize * 6]
                .iter()
                .all(|&b| b == b'x' || b == b'y')
        );
    }

    #[test]
    fn segments_are_sixteen_bytes() {
        // Упаковка ради памяти: сегмент — ровно четыре u32, без хвоста
        // выравнивания.
        assert_eq!(Buffer::segment_size(), 16);
        // Узел — тоже 16 байт: вид узла в старшем бите, дискриминант не нужен.
        assert_eq!(Buffer::node_size(), 16);
    }

    #[test]
    fn redo_text_is_reused_not_deleted() {
        // Тексты отменённых вставок не удаляются: `Added` обрезается по хвосту,
        // и следующая вставка перезаписывает то же место. Проверяем, что новая
        // вставка действительно легла поверх отменённой, а не после неё.
        let mut buf = b("");
        buf.insert(0, b"AAAA").unwrap();
        buf.insert(4, b"BBBB").unwrap();
        buf.insert(8, b"CCCC").unwrap();
        buf.undo().unwrap().unwrap(); // отменена "CCCC"
        buf.undo().unwrap().unwrap(); // отменена "BBBB"
        assert_eq!(buf.read(), b"AAAA");

        // Новая вставка обязана занять место отменённых, а не дописать после.
        buf.insert(4, b"ZZ").unwrap();
        assert_eq!(buf.read(), b"AAAAZZ");
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"AAAA");
        buf.redo().unwrap().unwrap();
        assert_eq!(buf.read(), b"AAAAZZ");
    }

    #[test]
    fn redo_text_reuse_shrinks_added_back() {
        // После отмены текст в `Added` остаётся: он нужен, если правка
        // вернётся redo. Обрезка случается только когда приходит новая правка —
        // тогда redo-ветка отменяется, а место переиспользуется.
        let mut buf = b("");
        for _ in 0..10 {
            buf.insert(buf.len(), b"0123456789").unwrap();
        }
        for _ in 0..10 {
            buf.undo().unwrap().unwrap();
        }
        assert!(buf.is_empty());
        assert!(
            !buf.redo_stack().is_empty(),
            "отменённые записи ждут в redo, текст из-за них не выбрасывается"
        );
        assert_eq!(
            buf.tree_memory().added_len,
            100,
            "текст отменённых вставок остаётся в Added ради redo"
        );

        // Новая правка: redo очищается, хвост срезается, место переиспользуется.
        let capacity_before = buf.tree_memory().added_capacity;
        buf.insert(0, b"x").unwrap();
        let mem = buf.tree_memory();
        assert_eq!(mem.added_len, 1, "хвост срезан до новой вставки");
        assert!(buf.redo_stack().is_empty());
        assert!(
            mem.added_capacity <= capacity_before,
            "ёмкость не должна расти: {} против {capacity_before}",
            mem.added_capacity
        );
        assert_eq!(buf.read(), b"x");
    }

    #[test]
    fn arena_blocks_keep_all_records() {
        // Глубина ограничивает историю: записей не больше заданного, лишнее
        // сворачивается снапшотом, текст при этом цел. Цепочка блоков арены
        // проверяется отдельно, напрямую на арене — через буфер до неё не дойти.
        const DEPTH: u32 = 2_048;
        let mut buf = Buffer::with_history_depth(b"", DEPTH).unwrap();
        for _ in 0..3000 {
            buf.insert(0, b"X").unwrap();
            assert!(buf.undo_stack().len() <= DEPTH as usize);
            for id in buf.undo_stack() {
                assert!(buf.record(id).is_some(), "запись пропала");
            }
        }
        assert_eq!(buf.read().len(), 3_000);
        assert!(buf.snapshots_taken() > 0);
        // Отменить можно только до последнего снапшота: дальше история уже
        // стёрта, поэтому исходный пустой документ недостижим.
        while buf.undo().unwrap().is_some() {}
        assert!(
            !buf.is_empty(),
            "откат должен упереться в снапшот, не в ноль"
        );
        assert!(
            buf.undo_stack().is_empty(),
            "откатились до конца своей истории"
        );
    }

    #[test]
    fn history_depth_boundaries() {
        // Глубина задаётся числом записей, отдельного выбора размера арены
        // нет.
        assert_eq!(Buffer::new(b"x").history_depth(), 8_192);
        assert_eq!(
            Buffer::with_history_depth(b"x", 1).unwrap().history_depth(),
            1
        );
        assert_eq!(
            Buffer::with_history_depth(b"x", 500)
                .unwrap()
                .history_depth(),
            500
        );
    }

    #[test]
    fn delete_physically_removes_segments() {
        let mut buf = Buffer::with_history_depth(b"0123456789", 2_048).unwrap();
        buf.delete(2, 4).unwrap();
        // Из чтения ушёл диапазон, сегментов больше нет — только живые куски.
        let segs = buf.segments();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].src(), crate::ORIGINAL);
        assert_eq!((segs[0].off, segs[0].len), (0, 2));
        assert_eq!((segs[1].off, segs[1].len), (6, 4));
        assert_eq!(buf.read(), b"016789");
    }

    #[test]
    fn delete_cuts_middle_of_single_segment() {
        // Удаление из середины исходника: сегмент разрезается на два живых куска.
        let mut buf = Buffer::with_history_depth(b"0123456789", 2_048).unwrap();
        buf.delete(3, 3).unwrap();
        let segs = buf.segments();
        assert_eq!(segs.len(), 2);
        assert_eq!((segs[0].off, segs[0].len), (0, 3));
        assert_eq!((segs[1].off, segs[1].len), (6, 4));
        assert_eq!(buf.read(), b"0126789");
    }

    #[test]
    fn delete_whole_document() {
        let mut buf = Buffer::with_history_depth(b"abc", 2_048).unwrap();
        buf.delete(0, 3).unwrap();
        assert!(buf.segments().is_empty());
        assert!(buf.is_empty());
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"abc");
    }

    #[test]
    fn huge_insert_is_not_limited_by_arena() {
        let mut buf = b("start|end");
        let big = vec![b'x'; 1_000_000];
        buf.insert(5, &big).unwrap();
        assert_eq!(buf.len(), 1_000_009);
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"start|end");
    }

    #[test]
    fn discarded_redo_texts_removed_from_arena() {
        let mut buf = b("");
        buf.insert(0, b"A").unwrap();
        buf.insert(1, b"B").unwrap();
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"A");
        let before = buf.undo_stack().len();
        buf.insert(1, b"C").unwrap();
        assert_eq!(buf.read(), b"AC");
        assert_eq!(buf.undo_stack().len(), before + 1);
        assert!(buf.redo_stack().is_empty());
    }

    #[test]
    fn differential_against_reference_string() {
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0
            }
            fn below(&mut self, n: usize) -> usize {
                (self.next() % n as u64) as usize
            }
        }

        let mut rng = Rng(0x7f4a7c15);
        let mut buf = Buffer::new(b"");
        let mut ref_text = String::new();

        for _ in 0..3000 {
            let data = [b'a' + (rng.below(26) as u8)];
            if ref_text.is_empty() {
                buf.insert(0, &data).unwrap();
                ref_text.insert(0, data[0] as char);
                continue;
            }
            if rng.below(3) == 0 {
                // Удаление.
                let anchor = rng.below(ref_text.len());
                let len = rng.below(ref_text.len() - anchor);
                buf.delete(anchor as u32, len as u32).unwrap();
                ref_text.drain(anchor..anchor + len);
            } else {
                // Вставка.
                let anchor = rng.below(ref_text.len() + 1);
                buf.insert(anchor as u32, &data).unwrap();
                ref_text.insert(anchor, data[0] as char);
            }
            assert_eq!(buf.read(), ref_text.as_bytes(), "состояния разошлись");
        }
    }

    // ── История переживает снапшот ─────────────────────────────────────────────

    /// Буфер со снапшотом, который помнит историю.
    fn with_keeper(
        text: &str,
    ) -> (
        Buffer,
        std::sync::Arc<std::sync::Mutex<Option<crate::history::History>>>,
    ) {
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut buf = Buffer::new(text.as_bytes());
        buf.set_sink("/tmp/doc.txt", Box::new(Shared(slot.clone())));
        (buf, slot)
    }

    /// Приёмник, пишущий в общую ячейку: `Keeper` живёт внутри буфера и наружу
    /// не отдаётся, а тесту история нужна после снапшота.
    struct Shared(std::sync::Arc<std::sync::Mutex<Option<crate::history::History>>>);

    impl std::fmt::Debug for Shared {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Shared")
        }
    }

    impl crate::snapshot::SnapshotSink for Shared {
        fn accept(
            &mut self,
            _file: &std::path::Path,
            _text: &[u8],
            history: &crate::history::History,
        ) {
            *self.0.lock().unwrap() = Some(history.clone());
        }
    }

    /// Приёмник, отдающий последнюю историю снапшота.
    #[derive(Debug, Clone)]
    struct Grab(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

    impl crate::snapshot::SnapshotSink for Grab {
        fn accept(
            &mut self,
            _file: &std::path::Path,
            _text: &[u8],
            history: &crate::history::History,
        ) {
            let mut slot = self.0.lock().unwrap();
            slot.push(history.encode());
        }
    }

    #[test]
    fn a_snapshot_carries_the_history_with_it() {
        // Главное: снапшот уносит историю. Текст после перезапуска верный, но без
        // истории отменять было бы нечего.
        let (mut buf, slot) = with_keeper("abc");
        buf.insert(1, b"XY").unwrap();
        buf.delete(0, 1).unwrap();
        let text_before = buf.read();

        buf.snapshot().unwrap();

        let history = slot.lock().unwrap().clone().expect("снапшот отдал историю");
        assert!(!history.is_empty(), "история снапшота пуста");

        let mut fresh = Buffer::new(b"");
        fresh.restore_history(&history).unwrap();
        assert_eq!(
            fresh.read(),
            text_before,
            "текст после восстановления разошёлся"
        );
    }

    #[test]
    fn restored_history_can_be_undone() {
        // Восстановленная история отменяется, как обычная: откат назад до
        // состояния до снапшота работает.
        let (mut buf, slot) = with_keeper("abc");
        buf.insert(3, b"XY").unwrap();
        buf.snapshot().unwrap();
        let history = slot.lock().unwrap().clone().expect("снапшот отдал историю");

        let mut fresh = Buffer::new(b"");
        fresh.restore_history(&history).unwrap();
        assert_eq!(fresh.read(), b"abcXY");

        fresh.undo().unwrap().expect("отмена доступна");
        assert_eq!(fresh.read(), b"abc");
    }

    #[test]
    fn restored_history_can_be_redone() {
        let (mut buf, slot) = with_keeper("abc");
        buf.insert(3, b"XY").unwrap();
        buf.undo().unwrap();
        buf.snapshot().unwrap();
        let history = slot.lock().unwrap().clone().expect("снапшот отдал историю");

        let mut fresh = Buffer::new(b"");
        fresh.restore_history(&history).unwrap();
        assert_eq!(fresh.read(), b"abc");
        fresh.redo().unwrap().expect("повтор доступен");
        assert_eq!(fresh.read(), b"abcXY");
    }

    #[test]
    fn an_empty_history_changes_nothing() {
        let mut fresh = Buffer::new(b"abc");
        fresh.restore_history(&crate::history::empty()).unwrap();
        assert_eq!(fresh.read(), b"abc");
        assert!(fresh.undo().unwrap().is_none(), "отменять нечего");
    }

    #[test]
    fn an_automatic_snapshot_carries_the_history_it_had() {
        // Арена кончилась, буфер снял снапшот сам. Унести он должен всё, что к
        // этому моменту было, иначе отменять после перезапуска будет нечего.
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        // Первый слот арены зарезервирован под `Original`, поэтому при глубине 4
        // полезных записей три, а не четыре.
        let mut buf = Buffer::with_history_depth(b"abc", 4).unwrap();
        buf.set_sink("/tmp/doc.txt", Box::new(Grab(slot.clone())));

        for i in 0..3 {
            buf.insert(i as u32, b"x").unwrap();
        }
        // Четвёртая правка: арена полна, снапшот снимается ДО неё.
        buf.insert(0, b"y").unwrap();

        let saved = slot.lock().unwrap().clone();
        assert_eq!(
            saved.len(),
            1,
            "автоснапшот должен быть один, получили {}",
            saved.len()
        );

        let history = crate::history::History::decode(&saved[0]);
        let mut fresh = Buffer::new(b"");
        fresh.restore_history(&history).unwrap();
        assert_eq!(fresh.read(), b"xxxabc", "в снапшоте должны быть три правки");

        // Все три записи на месте: откат доходит до исходного текста.
        let mut undone = 0;
        while fresh.undo().unwrap().is_some() {
            undone += 1;
        }
        assert_eq!(undone, 3, "восстановилось {} записей из 3", undone);
        assert_eq!(fresh.read(), b"abc");
    }

    #[test]
    fn a_file_without_history_still_opens() {
        // Старая база лежит без колонки истории: снапшот без неё должен
        // читаться, просто отменять в нём нечего.
        let history = crate::history::History::decode(&[]);
        assert!(history.is_empty());
        let mut buf = Buffer::new(b"abc");
        buf.restore_history(&history).unwrap();
        assert_eq!(buf.read(), b"abc");
        assert!(buf.undo().unwrap().is_none());
    }

    #[test]
    fn history_larger_than_the_arena_is_not_half_restored() {
        // Глубину истории уменьшили в конфиге: арена стала меньше, чем была при
        // записи. Часть записей в неё не поместится — и если молча обрезать,
        // документ получится не тот, что был. Лучше не восстанавливать вовсе.
        let (mut buf, slot) = with_keeper("abc");
        for i in 0..10 {
            buf.insert(i as u32, b"x").unwrap();
        }
        let text = buf.read();
        buf.snapshot().unwrap();
        let history = slot.lock().unwrap().clone().expect("снапшот отдал историю");
        assert_eq!(history.undo().len() / 13, 10);

        // Открываем с глубиной 4: десять записей не поместятся.
        let mut fresh = Buffer::with_history_depth(b"", 4).unwrap();
        fresh.restore_history(&history).unwrap();

        // Документ остаётся таким, каким был, и откатывается целиком.
        assert_eq!(
            fresh.read(),
            text,
            "документ разошёлся после уменьшения глубины"
        );
        let mut undone = 0;
        while fresh.undo().unwrap().is_some() {
            undone += 1;
        }
        assert_eq!(undone, 10, "восстановилось {} записей из 10", undone);
        assert_eq!(fresh.read(), b"abc");
    }
}

/// Внутренние инварианты буфера и неподвижные проверки.
///
/// Обычные тесты проверяют текст. Здесь проверяется то, что иначе может тихо
/// разъехаться: целостность пула узлов, баланс дерева, сходимость частичного
/// чтения с полным, поведение арены на границах блоков и соответствие эталону
/// на длинных случайных последовательностях.
#[cfg(test)]
mod invariants {
    use crate::arena::{Arena, Record, RecordId};
    use crate::buffer::Buffer;
    use crate::buffer::Error;

    /// Короткий конструктор буфера из строки.
    fn b(text: &str) -> Buffer {
        Buffer::new(text.as_bytes())
    }

    /// Максимальная высота AVL-дерева с `leaves` листьями.
    ///
    /// Минимум листьев для высоты h равен F(h+1): F(1)=1, F(2)=1, F(3)=2… —
    /// это в точности рекурсия F(h) = F(h-1) + F(h-2) из определения баланса.
    fn max_avl_height(leaves: usize) -> usize {
        // fib[k] — число листьев, достаточное для высоты k.
        let (mut prev, mut cur) = (0usize, 1usize); // F(0), F(1)
        let mut height = 0usize;
        // F(height+1) <= leaves
        while cur <= leaves {
            height += 1;
            let next = prev + cur;
            prev = cur;
            cur = next;
        }
        height
    }

    /// Проверить, что дерево цело: листья, баланс и учёт узлов сходятся.
    fn check_tree(buf: &Buffer, label: &str) {
        let leaves = buf.segments().len();
        let height = buf.tree_height();

        // Полное двоичное дерево: внутренних узлов ровно на один меньше, чем
        // листьев. Любая утечка или двойное освобождение узла это ломает.
        let mem = buf.tree_memory();
        let used = mem.nodes;
        let expected = if leaves == 0 { 0 } else { 2 * leaves - 1 };
        assert_eq!(
            used, expected,
            "{label}: занято {used} узлов, листьев {leaves}, ожидалось {expected}"
        );

        // Высота AVL-дерева ограничена: минимум листьев для высоты h — это
        // числа Фибоначчи, F(h+1). Логарифмическая оценка 1.44*log2(n) верна
        // только для больших n и на малых ошибается, поэтому считаем точно.
        if leaves > 0 {
            let max_height = max_avl_height(leaves);
            assert!(
                height <= max_height,
                "{label}: высота {height} при {leaves} листьях превышает {max_height}"
            );
            assert!(height >= 1);
        } else {
            assert_eq!(height, 0, "{label}: пустое дерево должно иметь высоту 0");
        }

        // Лист не может быть пустым: пустой сегмент означал бы, что о нём
        // забыли и он навсегда остался бы в дереве.
        for (i, s) in buf.segments().iter().enumerate() {
            assert!(s.len > 0, "{label}: пустой сегмент #{i}");
        }
    }

    /// Генератор чисел: детерминированный, чтобы падение воспроизводилось.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                return 0;
            }
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    #[test]
    fn tree_accounting_holds_under_random_edits() {
        for seed in 0..8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));
            let mut buf = Buffer::new(b"start");
            let mut text = String::from("start");
            for step in 0..400 {
                if text.is_empty() || rng.below(3) != 0 {
                    let pos = rng.below(text.len() + 1);
                    let piece: String = (0..rng.below(4) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    buf.insert(pos as u32, piece.as_bytes()).unwrap();
                    text.insert_str(pos, &piece);
                } else {
                    let pos = rng.below(text.len());
                    let len = rng.below(text.len() - pos);
                    buf.delete(pos as u32, len as u32).unwrap();
                    text.drain(pos..pos + len);
                }
                assert_eq!(buf.read(), text.as_bytes(), "seed {seed} step {step}");
                assert_eq!(buf.len() as usize, text.len());
                check_tree(&buf, &format!("seed {seed} step {step}"));
            }
        }
    }

    #[test]
    fn tree_accounting_holds_after_undo_and_redo_walk() {
        let mut rng = Rng(0xC0FFEE);
        let mut buf = Buffer::new(b"");
        let mut history = 0usize;

        for _ in 0..600 {
            if rng.below(4) != 0 {
                let pos = rng.below(buf.len() as usize + 1) as u32;
                let piece: Vec<u8> = (0..rng.below(5) + 1)
                    .map(|_| b'a' + rng.below(26) as u8)
                    .collect();
                buf.insert(pos, &piece).unwrap();
                history += 1;
            } else if !buf.is_empty() {
                let pos = rng.below(buf.len() as usize) as u32;
                let len = rng.below((buf.len() - pos) as usize) as u32 + 1;
                buf.delete(pos, len).unwrap();
                history += 1;
            }
            check_tree(&buf, "после правки");
        }

        // Отматываем всё до нуля: дерево обязано схлопнуться без остатка.
        let mut undone = 0;
        while buf.undo().unwrap().is_some() {
            undone += 1;
            check_tree(&buf, "после undo");
            assert!(undone <= history);
        }
        assert_eq!(undone, history, "отменилось не всё");
        assert_eq!(buf.len(), 0, "документ обязан вернуться к исходному");

        // И возвращаем всё обратно.
        let mut redone = 0;
        while buf.redo().unwrap().is_some() {
            redone += 1;
            check_tree(&buf, "после redo");
        }
        assert_eq!(redone, history);
    }

    #[test]
    fn every_slice_matches_full_read() {
        // Полное пересечение срезов: для каждой пары границ частичное чтение
        // обязано совпасть с подстрокой полного. Ловит ошибки баз в рекурсии,
        // которые выборочные проверки пропускают.
        let mut buf = Buffer::new(b"");
        for i in 0..60u8 {
            buf.insert(buf.len(), &[b'a' + i]).unwrap();
        }
        buf.insert(10, b"MIDDLE").unwrap();
        buf.delete(20, 5).unwrap();

        let full = buf.read();
        let n = full.len();
        for start in 0..=n {
            for end in start..=n {
                assert_eq!(
                    buf.read_range(start as u32, end as u32),
                    full[start..end],
                    "срез {start}..{end}"
                );
            }
        }
    }

    #[test]
    fn read_range_clamps_out_of_bounds() {
        let buf = b("0123456789");
        assert_eq!(buf.read_range(5, 5), b"");
        assert_eq!(buf.read_range(0, 1000), b"0123456789");
        assert_eq!(buf.read_range(1000, 2000), b"");
        assert!(buf.read_range(500, 100).is_empty());
    }

    #[test]
    fn empty_document_behaves() {
        let mut buf = Buffer::new(b"");
        assert!(buf.is_empty());
        assert_eq!(buf.read(), b"");
        assert_eq!(buf.tree_height(), 0);
        check_tree(&buf, "пустой");

        assert!(buf.delete(0, 1).is_err());
        assert!(buf.insert(1, b"x").is_err());
        buf.insert(0, b"x").unwrap();
        assert_eq!(buf.read(), b"x");
        check_tree(&buf, "после вставки в пустой");

        buf.delete(0, 1).unwrap();
        assert!(buf.is_empty());
        check_tree(&buf, "после удаления всего");
    }

    #[test]
    fn insert_and_delete_at_every_boundary() {
        let mut buf = Buffer::new(b"");
        buf.insert(0, b"0123456789").unwrap();
        let full = buf.read();
        let n = full.len() as u32;

        // Вставка в каждую возможную точку.
        for pos in 0..=n {
            let mut b2 = Buffer::new(&full[..]);
            b2.insert(pos, b"#").unwrap();
            let mut want = full.clone();
            want.insert(pos as usize, b'#');
            assert_eq!(b2.read(), want, "вставка в {pos}");
            check_tree(&b2, &format!("вставка в {pos}"));
        }

        // Удаление каждого возможного диапазона.
        for pos in 0..n {
            for len in 1..=(n - pos) {
                let mut b2 = Buffer::new(&full[..]);
                b2.delete(pos, len).unwrap();
                let mut want = full.clone();
                want.drain(pos as usize..(pos + len) as usize);
                assert_eq!(b2.read(), want, "удаление {pos}..{}", pos + len);
                check_tree(&b2, &format!("удаление {pos}..{}", pos + len));
            }
        }
    }

    #[test]
    fn arena_chains_blocks_without_losing_records() {
        // Арена не теряет записи: блок переполнен — выделяется следующий. Через
        // буфер этого не увидеть, потому что буфер снимает снапшот ровно
        // тогда, когда глубина исчерпана, и до второго блока не доходит.
        // Поэтому здесь арена проверяется напрямую.
        let mut arena = Arena::new(64);
        let mut ids: Vec<RecordId> = Vec::new();
        for i in 0..1_000u32 {
            ids.push(arena.push(Record::insert(i, 1, i)));
        }
        assert!(arena.block_count() > 1, "блоков {}", arena.block_count());
        assert_eq!(arena.len(), 1_000, "записи теряться не должны");

        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "идентификаторы повторились");

        // Все записи читаются и адреса согласованы.
        for (i, &id) in ids.iter().enumerate() {
            let rec = arena.get(id).expect("запись пропала");
            assert_eq!(rec.anchor(), i as u32);
            assert_eq!(rec.len(), 1);
        }

        // Сброс возвращает арену к первому блоку, переиспользуя его буфер.
        arena.reset();
        assert_eq!(arena.len(), 0);
        assert!(arena.path().is_empty());
        assert_eq!(
            arena.push(Record::delete(0, 1, 0)),
            1,
            "нумерация начинается заново"
        );
    }

    #[test]
    fn record_ids_restart_after_snapshot() {
        // Снапшот очищает арену, поэтому идентификаторы начинаются заново. Это
        // безопасно: дерево схлопнуто в новый Original и ссылок на старые
        // записи не остаётся.
        let mut buf = Buffer::with_history_depth(b"", 100).unwrap();
        let mut saw_restart = false;
        let mut prev = 0u32;
        for _ in 0..600 {
            buf.insert(buf.len(), b"x").unwrap();
            let id = *buf.undo_stack().last().unwrap();
            if id <= prev {
                saw_restart = true;
            }
            prev = id;
            assert!(buf.undo_stack().len() <= 100, "глубина превышена");
        }
        assert!(
            saw_restart,
            "после снапшота нумерация обязана начаться заново"
        );
        // Текст при этом цел.
        assert_eq!(buf.read().len(), 600);
        check_tree(&buf, "после серии снапшотов");
    }

    #[test]
    fn record_len_and_text_accessors() {
        let mut buf = b("");
        buf.insert(0, b"abc").unwrap();
        let id = *buf.undo_stack().last().unwrap();
        let rec = buf.record(id).unwrap();
        assert_eq!(rec.anchor(), 0);
        assert_eq!(rec.head(), 3);
        assert_eq!(rec.len(), 3);
        assert!(!rec.is_empty());
        assert!(rec.text().is_some(), "вставка несёт ссылку на текст");

        buf.delete(0, 1).unwrap();
        let id = *buf.undo_stack().last().unwrap();
        let rec = buf.record(id).unwrap();
        assert_eq!(rec.len(), 1);
        assert!(
            rec.text().is_some(),
            "удаление помнит вырезанный текст: без него отменять нечем"
        );
    }

    #[test]
    fn history_depth_bounds_records_and_leaves() {
        // Листья ничем не ограничены, ограничена глубина истории. Записей в
        // арене не может быть больше глубины, а листьев — больше
        // `2 · depth + 1`.
        for depth in [1u32, 7, 100, 1_000] {
            let mut buf = Buffer::with_history_depth(b"", depth).unwrap();
            let mut pos = 0u32;
            for i in 0..(depth as usize * 5) {
                pos = (pos + 13) % buf.len().max(1);
                buf.insert(pos, b"payload").unwrap();
                if i % 3 == 0 && buf.len() > 20 {
                    buf.delete(pos, 3).unwrap();
                }
                assert!(
                    buf.undo_stack().len() <= depth as usize,
                    "глубина {depth}: записей {} при глубине {depth}",
                    buf.undo_stack().len()
                );
                assert!(
                    buf.segments().len() <= 2 * depth as usize + 1,
                    "глубина {depth}: листьев {} при пределе {}",
                    buf.segments().len(),
                    2 * depth as usize + 1
                );
            }
            assert!(buf.snapshots_taken() > 0, "снапшот обязан был сработать");
            check_tree(&buf, &format!("глубина {depth}"));
        }
    }

    #[test]
    fn messages_are_localizable_and_default_to_english() {
        use crate::messages::{En, Messages, substitute};

        // По умолчанию — английский.
        let out_of_bounds = Error::OutOfBounds {
            anchor: 10,
            len: 5,
            doc_len: 4,
        };
        let text = out_of_bounds.text();
        assert!(text.contains("outside"), "по умолчанию английский: {text}");
        assert!(
            !text.contains("вне"),
            "русского в дефолте быть не должно: {text}"
        );

        // Каталог из конфига: текст написан переводчиком, значения подставляются
        // вместо имён в фигурных скобках.
        struct Ru;
        impl Messages for Ru {
            fn out_of_bounds(&self, anchor: u32, end: u32, doc_len: u32) -> String {
                substitute(
                    "диапазон {anchor}..{end} вне документа длиной {doc_len}",
                    &[
                        ("anchor", anchor.to_string()),
                        ("end", end.to_string()),
                        ("doc_len", doc_len.to_string()),
                    ],
                )
            }
            fn invalid_history_depth(&self, depth: u32, min: u32, max: u32) -> String {
                substitute(
                    "глубина истории {depth} недопустима: от {min} до {max}",
                    &[
                        ("depth", depth.to_string()),
                        ("min", min.to_string()),
                        ("max", max.to_string()),
                    ],
                )
            }
            fn tree_pool_exhausted(&self, capacity: usize, depth: u32) -> String {
                substitute(
                    "пул исчерпан: узлов {capacity}, глубина {depth}",
                    &[
                        ("capacity", capacity.to_string()),
                        ("depth", depth.to_string()),
                    ],
                )
            }
            fn bad_placeholder(&self, got: &str, expected: &[&str]) -> String {
                substitute(
                    "в переводе имя {{{got}}}, а ожидалось одно из: {expected}",
                    &[("got", got.to_string()), ("expected", expected.join(", "))],
                )
            }
            fn unclosed_brace(&self, tail: &str) -> String {
                substitute(
                    "в переводе не закрыта скобка: {tail}",
                    &[("tail", tail.to_string())],
                )
            }
        }

        let ru = out_of_bounds.message(&Ru);
        assert_eq!(ru, "диапазон 10..15 вне документа длиной 4");

        // Английский каталог даёт английский текст, и это тот же самый.
        assert_eq!(out_of_bounds.message(&En), text);

        // Все три ошибки собираются через любой каталог.
        let depth_err = Error::InvalidHistoryDepth { depth: 0 };
        assert_eq!(
            depth_err.message(&Ru),
            "глубина истории 0 недопустима: от 1 до 8192"
        );
        let pool_err = Error::TreePoolExhausted {
            capacity: 408,
            depth: 100,
        };
        assert_eq!(
            pool_err.message(&Ru),
            "пул исчерпан: узлов 408, глубина 100"
        );
        assert!(pool_err.message(&En).contains("exhausted"));
    }

    #[test]
    fn every_default_is_present_and_documented() {
        use crate::defaults::buffer;

        // Ключ не пустой, комментарий полный — у всех настроек.
        let (name, key, comment) = (
            "Buffer.history_depth",
            buffer::HISTORY_DEPTH.key,
            buffer::HISTORY_DEPTH.comment,
        );
        assert!(!key.is_empty(), "{name}: ключ не может быть пустым");
        assert!(
            !comment.is_empty(),
            "{name}: без комментария непонятно, что это"
        );
        assert!(
            comment.contains('.'),
            "{name}: комментарий должен быть полным"
        );

        // Какая настройка подключена, а какая задумана — проверяется при
        // компиляции.
        //
        // Помечено `used` только то, что код действительно читает. Флаг врёт
        // легко: настройка выглядит подключённой, а её значение никто не
        // открывает. Тогда пользователь правит конфиг и не видит никакой
        // разницы.
        const _: () = assert!(buffer::HISTORY_DEPTH.active);

        // Значения по умолчанию согласованы с настоящими ограничениями.
        // Значение по умолчанию обязано быть тем, с чем реально создаётся буфер.
        // Иначе настройка и поведение разойдутся.
        assert_eq!(buffer::HISTORY_DEPTH.value, crate::DEFAULT_DEPTH);
        const _: () = assert!(buffer::HISTORY_DEPTH.value <= crate::MAX_DEPTH);
        const _: () = assert!(
            buffer::HISTORY_DEPTH.value <= crate::MAX_DEPTH,
            "глубина по умолчанию должна быть в допустимых пределах"
        );
    }

    #[test]
    fn bad_placeholder_names_what_came_and_what_was_expected() {
        use crate::messages::substitute;
        use crate::messages::{Messages, ParseError};

        struct Ru;
        impl Messages for Ru {
            fn out_of_bounds(&self, _a: u32, _e: u32, _d: u32) -> String {
                String::new()
            }
            fn invalid_history_depth(&self, _d: u32, _min: u32, _max: u32) -> String {
                String::new()
            }
            fn tree_pool_exhausted(&self, _c: usize, _d: u32) -> String {
                String::new()
            }
            fn bad_placeholder(&self, got: &str, expected: &[&str]) -> String {
                substitute(
                    "в переводе имя {{{got}}}, а ожидалось одно из: {expected}",
                    &[("got", got.to_string()), ("expected", expected.join(", "))],
                )
            }
            fn unclosed_brace(&self, tail: &str) -> String {
                substitute(
                    "в переводе не закрыта скобка: {tail}",
                    &[("tail", tail.to_string())],
                )
            }
        }

        // Опечатка в имени: показываем, что пришло и чего ждали.
        let unknown = ParseError::UnknownName {
            got: "capasity",
            expected: &["capacity", "depth"],
        };
        let en = unknown.text();
        assert!(
            en.contains("capasity"),
            "в сообщении должно быть, что пришло: {en}"
        );
        assert!(
            en.contains("capacity"),
            "в сообщении должен быть ожидаемый список: {en}"
        );
        assert!(
            en.contains("depth"),
            "в сообщении должен быть ожидаемый список: {en}"
        );

        let ru = unknown.message(&Ru);
        assert!(ru.contains("capasity"), "русский каталог: {ru}");
        assert!(ru.contains("capacity"), "русский каталог: {ru}");

        // Незакрытая скобка: показываем хвост начиная с неё.
        let unclosed = ParseError::UnclosedBrace { tail: "{capacity" };
        assert!(
            unclosed.text().contains("{capacity"),
            "хвост должен быть виден"
        );
        assert!(unclosed.message(&Ru).contains("не закрыта"));
    }

    #[test]
    fn zero_and_overlarge_depth_report_clear_errors() {
        // Ноль бессмыслен: ни одна правка не поместилась бы. Ошибка, а не
        // паника: значение приходит из конфига, и редактор должен сказать
        // почему.
        let err = Buffer::with_history_depth(b"", 0).expect_err("глубина 0 обязана отвергаться");
        assert_eq!(err, Error::InvalidHistoryDepth { depth: 0 });
        let text = err.to_string();
        assert!(
            text.contains('0'),
            "в сообщении должно быть само значение: {text}"
        );
        assert!(
            text.contains('1'),
            "в сообщении должен быть диапазон: {text}"
        );

        // Выше предела — тоже ошибка, а не паника.
        let over = crate::MAX_DEPTH + 1;
        let err = Buffer::with_history_depth(b"", over).expect_err("слишком глубокая история");
        assert_eq!(err, Error::InvalidHistoryDepth { depth: over });

        // Границы диапазона принимаются.
        assert!(Buffer::with_history_depth(b"", 1).is_ok());
        assert!(Buffer::with_history_depth(b"", crate::MAX_DEPTH).is_ok());
    }

    #[test]
    fn node_pool_is_fixed_size_and_never_grows() {
        // Пул выделяется один раз под глубину и больше не растёт. Ёмкость
        // выведена из неё: одна запись даёт максимум два листа, полное двоичное
        // дерево на столько листьев занимает `4 · depth + 1` узлов.
        for depth in [1u32, 100, 1_000, 8_192] {
            let buf = Buffer::with_history_depth(b"tiny", depth).unwrap();
            let mem = buf.tree_memory();
            assert_eq!(
                mem.pool_capacity,
                4 * depth as usize + 8,
                "глубина {depth}: неверная ёмкость пула"
            );
            assert_eq!(buf.history_depth(), depth);
        }
        // На пределе: 32776 узлов по 16 байт — это 512 КБ.
        assert_eq!(Buffer::new(b"x").tree_memory().tree_bytes(), 32_776 * 16);
    }

    #[test]
    fn node_pool_reused_across_churn() {
        // Волны вставок и удалений не должны раздувать пул: узлы возвращаются
        // в свободные и берутся снова.
        let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
        let pool = buf.tree_memory().pool_capacity;
        for _ in 0..5_000 {
            buf.insert(buf.len(), b"chunk").unwrap();
        }
        assert_eq!(buf.len(), 25_000);
        assert_eq!(buf.tree_memory().pool_capacity, pool, "пул изменился");
        let peak_nodes = buf.tree_memory().nodes;

        while !buf.is_empty() {
            buf.delete(0, 1).unwrap();
        }
        check_tree(&buf, "после полного удаления");

        for _ in 0..5_000 {
            buf.insert(buf.len(), b"chunk").unwrap();
        }
        let now_nodes = buf.tree_memory().nodes;
        assert!(
            now_nodes <= peak_nodes + 16,
            "пул вырос: {now_nodes} против {peak_nodes}"
        );
        assert_eq!(buf.read().len(), 25_000);
    }

    #[test]
    fn snapshot_clears_history_and_keeps_text() {
        let mut buf = b("hello");
        buf.insert(5, b" world").unwrap();
        buf.insert(11, b"!").unwrap();
        assert_eq!(buf.read(), b"hello world!");

        let before = buf.snapshots_taken();
        buf.snapshot().unwrap();
        assert_eq!(buf.snapshots_taken(), before + 1);
        assert_eq!(buf.read(), b"hello world!");
        assert_eq!(buf.original(), b"hello world!");
        assert!(buf.undo_stack().is_empty());
        assert!(buf.redo_stack().is_empty());
        assert!(buf.undo().unwrap().is_none(), "истории после снапшота нет");
        check_tree(&buf, "после снапшота");

        // Дальше буфер работает как обычно.
        buf.insert(12, b"?").unwrap();
        assert_eq!(buf.read(), b"hello world!?");
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"hello world!");
    }

    /// Что случилось с текстом — чтобы уметь откатить и вернуть.
    enum Op {
        Insert { pos: usize, text: Vec<u8> },
        Delete { pos: usize, removed: Vec<u8> },
    }

    #[test]
    fn matches_reference_with_undo_redo_mix() {
        for seed in 0..12u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(7));
            let mut buf = Buffer::new(b"seed");
            let mut text = String::from("seed");
            let mut steps: Vec<Op> = Vec::new();
            // Курсор в истории: всё до него применено, всё после — redo.
            let mut cursor = 0usize;

            for step in 0..1500 {
                let roll = rng.below(10);
                if roll < 5 && !text.is_empty() {
                    // Новая правка: redo-ветка отменяется, новая операция в
                    // конец. Позиция строго меньше длины: удалять нечего на самом
                    // краю.
                    let pos = rng.below(text.len());
                    let len = rng.below((text.len() - pos).min(40)) + 1;
                    let removed = text.as_bytes()[pos..pos + len].to_vec();
                    buf.delete(pos as u32, len as u32).unwrap();
                    text.replace_range(pos..pos + len, "");
                    steps.truncate(cursor);
                    steps.push(Op::Delete { pos, removed });
                    cursor = steps.len();
                } else if roll < 8 {
                    // Вставка.
                    let pos = rng.below(text.len() + 1);
                    let piece: String = (0..rng.below(30) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    buf.insert(pos as u32, piece.as_bytes()).unwrap();
                    text.insert_str(pos, &piece);
                    steps.truncate(cursor);
                    steps.push(Op::Insert {
                        pos,
                        text: piece.into_bytes(),
                    });
                    cursor = steps.len();
                } else if roll == 8 && cursor > 0 {
                    // Отмена.
                    cursor -= 1;
                    buf.undo()
                        .unwrap()
                        .expect("история должна была быть непустой");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => text.replace_range(*pos..*pos + t.len(), ""),
                        Op::Delete { pos, removed } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(removed))
                        }
                    }
                } else if cursor < steps.len() {
                    // Повтор.
                    buf.redo().unwrap().expect("redo должен был быть доступен");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(t))
                        }
                        Op::Delete { pos, removed } => {
                            text.replace_range(*pos..*pos + removed.len(), "")
                        }
                    }
                    cursor += 1;
                }

                assert_eq!(
                    buf.read(),
                    text.as_bytes(),
                    "seed {seed} step {step}: текст разошёлся"
                );
                assert_eq!(buf.len() as usize, text.len(), "seed {seed} step {step}");
                assert_eq!(buf.undo_stack().len(), cursor, "seed {seed} step {step}");
            }

            // Финальная сверка: отменяем всё до старта и возвращаем обратно.
            while buf.undo().unwrap().is_some() {
                cursor -= 1;
            }
            assert_eq!(cursor, 0);
            assert_eq!(buf.read(), b"seed", "seed {seed}: не вернулся к началу");

            while buf.redo().unwrap().is_some() {
                cursor += 1;
            }
            assert_eq!(cursor, steps.len());
            assert_eq!(
                buf.read(),
                text.as_bytes(),
                "seed {seed}: не вернулся в конец"
            );
        }
    }

    #[test]
    fn replace_matches_delete_then_insert() {
        let mut buf = b("The quick brown fox");
        buf.replace(4, 5, b"slow").unwrap();
        assert_eq!(buf.read(), b"The slow brown fox");
        // Первая отмена убирает вставку — остаётся документ без "quick".
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"The  brown fox");
        // Вторая отмена возвращает удалённое.
        buf.undo().unwrap().unwrap();
        assert_eq!(buf.read(), b"The quick brown fox");
        check_tree(&buf, "после replace и отмен");
    }
}

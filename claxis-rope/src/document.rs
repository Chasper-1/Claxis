//! Документ: текст, история и снапшоты.
//!
//! Функционал тот же, что у `claxis-buffer`: вставка, удаление, замена,
//! отмена, повтор, чтение, снапшот. Отличается только устройство: текст лежит
//! в одном буфере, а лист дерева — диапазон в нём, поэтому лист не
//! привязан к границам правок и одна вставка даёт один лист, а не три.

use std::path::PathBuf;

use crate::node::{NIL, Node};
use crate::tree::Tree;

/// Запись правки. Операция несётся самой записью.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Record {
    /// Вставка: куда, сколько и где текст.
    Insert { anchor: u32, head: u32, text: u32 },
    /// Удаление: что вырезано и где текст.
    Delete { anchor: u32, head: u32, text: u32 },
}

impl Record {
    /// Длина затронутого диапазона.
    pub fn len(&self) -> u32 {
        self.anchor().abs_diff(self.head())
    }

    /// Затронут ли диапазон.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Начало диапазона.
    pub fn anchor(&self) -> u32 {
        match self {
            Record::Insert { anchor, .. } | Record::Delete { anchor, .. } => *anchor,
        }
    }

    /// Конец диапазона.
    pub fn head(&self) -> u32 {
        match self {
            Record::Insert { head, .. } | Record::Delete { head, .. } => *head,
        }
    }

    /// Смещение текста в буфере.
    pub fn text(&self) -> u32 {
        match self {
            Record::Insert { text, .. } | Record::Delete { text, .. } => *text,
        }
    }
}

/// Вершина истории: запись плюс родитель и ребёнок.
#[derive(Clone, Copy, Debug)]
struct Version {
    record: Record,
    parent: u32,
    child: u32,
}

/// Что пошло не так.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Диапазон вышел за пределы документа.
    OutOfBounds { anchor: u32, len: u32, doc_len: u32 },
    /// Глубина истории вне диапазона `min..`.
    ///
    /// Потолка нет: история растёт вместе с правками и память добирается по
    /// мере надобности. Отвергается только ноль — с ним снимался бы снапшот
    /// после каждой правки.
    InvalidHistoryDepth { depth: u32 },
    /// В истории лежит не формат rope: прежний буфер писал в ту же колонку
    /// свой блоб, и разбирать его как родной нельзя.
    NotOurHistory,
    /// Версия истории не та, что ждёт крейт.
    UnknownHistoryVersion {
        /// Что пришло.
        version: u8,
    },
    /// История оборвана: данных меньше, чем записано в заголовке.
    TruncatedHistory,
    /// Запись истории неизвестного вида.
    UnknownRecordKind {
        /// Что пришло в байте вида.
        kind: u8,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    ///
    /// Крейт отдаёт структуру и передаёт каталогу значения. Как они попадут в
    /// текст — дело каталога: в переводе сообщение пишется целиком, с
    /// подстановками в тех местах, куда их поставил переводчик.
    pub fn message(&self, messages: &dyn crate::messages::Messages) -> String {
        match self {
            Error::OutOfBounds {
                anchor,
                len,
                doc_len,
            } => messages.out_of_bounds(*anchor, anchor.saturating_add(*len), *doc_len),
            Error::InvalidHistoryDepth { depth } => messages.invalid_history_depth(*depth, 1),
            Error::NotOurHistory => messages.not_our_history(),
            Error::UnknownHistoryVersion { version } => messages.unknown_history_version(*version),
            Error::TruncatedHistory => messages.truncated_history(),
            Error::UnknownRecordKind { kind } => messages.unknown_record_kind(*kind),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for Error {}

/// Результат операции с документом.
pub type Result<T> = std::result::Result<T, Error>;

/// Глубина истории по умолчанию: сколько правок живёт до снапшота.
///
/// Потолка нет: история растёт по мере надобности, а это число только
/// говорит, когда пора снять снапшот и отдать память обратно.
pub const DEFAULT_DEPTH: u32 = 8 * 1024;

/// Минимальная глубина истории.
pub const MIN_DEPTH: u32 = 1;

/// Снимок памяти — для диагностики и тестов.
#[derive(Clone, Copy, Debug)]
pub struct Memory {
    /// Занято узлов.
    pub nodes: usize,
    /// Свободно узлов.
    pub free: usize,
    /// Ёмкость пула.
    pub capacity: usize,
    /// Листьев.
    pub leaves: usize,
    /// Размер узла.
    pub node_size: usize,
    /// Байт в буфере текста.
    pub text_len: usize,
    /// Ёмкость буфера текста.
    pub text_capacity: usize,
    /// Вершин истории.
    pub versions: usize,
    /// Размер вершины.
    pub version_size: usize,
}

impl Memory {
    /// Байт в дереве.
    pub fn tree_bytes(&self) -> usize {
        self.capacity * self.node_size
    }
}

/// Куда уходит снятый снапшот.
pub trait SnapshotSink: std::fmt::Debug {
    /// Документ снят: сохранить текст и историю.
    ///
    /// Ошибка не возвращается: потеря кеша не должна стоять пользователю
    /// работы.
    fn accept(&mut self, path: &std::path::Path, text: &[u8], history: &[u8]);
}

/// Приёмник, который ничего не делает.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;

impl SnapshotSink for NullSink {
    fn accept(&mut self, _path: &std::path::Path, _text: &[u8], _history: &[u8]) {}
}

/// Документ: текст, история и снапшоты.
pub struct Document {
    /// Весь текст: исходный файл в начале, дальше вставки и вырезанное.
    text: Vec<u8>,
    /// Граница исходного файла: вставки идут после неё.
    file_len: u32,
    tree: Tree,
    versions: Vec<Version>,
    free: Vec<u32>,
    current: u32,
    /// Глубина истории.
    depth: u32,
    /// Смещения текста, вставленного отменой удаления.
    ///
    /// Такой текст не принадлежит ни одной записи: запись удаления лежит в
    /// отброшенной ветке, и её обрезка унесла бы текст, на который уже
    /// ссылается живой лист. Хвост буфера не режется ниже этих точек.
    kept: Vec<u32>,
    sink: Box<dyn SnapshotSink>,
    file: Option<PathBuf>,
    snapshots: u64,
}

impl Document {
    /// Документ с текстом и глубиной истории по умолчанию.
    pub fn new(text: impl AsRef<[u8]>) -> Self {
        Self::with_depth(text, DEFAULT_DEPTH).expect("глубина по умолчанию верна")
    }

    /// Документ с заданной глубиной истории.
    pub fn with_depth(text: impl AsRef<[u8]>, depth: u32) -> Result<Self> {
        if depth < MIN_DEPTH {
            return Err(Error::InvalidHistoryDepth { depth });
        }
        let text = text.as_ref().to_vec();
        let len = text.len() as u32;
        Ok(Self::assemble(text, len, depth))
    }

    /// Документ из готового буфера: текст уже состоит из файла и хвоста.
    ///
    /// Основание — первые `file_len` байт, остальное хвост: так записи
    /// получают верные смещения, а документ остаётся ровно файлом.
    pub fn from_buffer(text: Vec<u8>, file_len: usize) -> Self {
        let file_len = file_len.min(text.len());
        let file_len = file_len as u32;
        Self::assemble(text, file_len, DEFAULT_DEPTH)
    }

    fn assemble(text: Vec<u8>, file_len: u32, depth: u32) -> Self {
        Self {
            tree: Tree::with_leaf(0, file_len),
            text,
            file_len,
            versions: vec![Version {
                record: Record::Delete {
                    anchor: 0,
                    head: 0,
                    text: 0,
                },
                parent: NIL,
                child: NIL,
            }],
            free: Vec::new(),
            current: 0,
            depth,
            kept: Vec::new(),
            sink: Box::new(NullSink),
            file: None,
            snapshots: 0,
        }
    }

    /// Весь текстовый буфер: файл и хвост вставок и вырезанного.
    pub fn buffer(&self) -> &[u8] {
        &self.text
    }

    /// Длина текстового буфера.
    pub fn buffer_len(&self) -> usize {
        self.text.len()
    }

    /// Длина текста файла: всё, что стоит до хвоста.
    pub fn file_len(&self) -> u32 {
        self.file_len
    }

    /// Привязать снапшоты к файлу и приёмнику.
    pub fn set_sink(&mut self, file: impl Into<PathBuf>, sink: Box<dyn SnapshotSink>) {
        self.file = Some(file.into());
        self.sink = sink;
    }

    /// Глубина истории.
    pub fn history_depth(&self) -> u32 {
        self.depth
    }

    /// Длина документа.
    pub fn len(&self) -> u32 {
        self.tree.len
    }

    /// Пуст ли документ.
    pub fn is_empty(&self) -> bool {
        self.tree.len == 0
    }

    fn check(&self, anchor: u32, len: u32) -> Result<()> {
        if anchor as u64 + len as u64 > self.tree.len as u64 {
            return Err(Error::OutOfBounds {
                anchor,
                len,
                doc_len: self.tree.len,
            });
        }
        Ok(())
    }

    // ── правки ───────────────────────────────────────────────────────────

    /// Вставить текст на позицию `anchor`.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) -> Result<()> {
        self.check(anchor, 0)?;
        if data.is_empty() {
            return Ok(());
        }
        self.discard_redo();
        let start = self.text.len() as u32;
        self.text.extend_from_slice(data);
        let len = data.len() as u32;
        let record = Record::Insert {
            anchor,
            head: anchor + len,
            text: start,
        };
        self.apply(record);
        Ok(())
    }

    /// Вырезать диапазон `anchor..anchor+len`.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<()> {
        self.check(anchor, len)?;
        if len == 0 {
            return Ok(());
        }
        self.discard_redo();
        // Вырезанное копируется в хвост буфера: иначе отменять нечем, а
        // второго буфера быть не должно.
        let start = self.text.len() as u32;
        let piece = self.slice(anchor, anchor + len);
        self.text.extend_from_slice(&piece);
        let record = Record::Delete {
            anchor,
            head: anchor + len,
            text: start,
        };
        self.apply(record);
        Ok(())
    }

    /// Заменить диапазон: удаление и вставка.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<()> {
        self.delete(anchor, len)?;
        self.insert(anchor, data)
    }

    /// Отменить одну правку обратной операцией.
    pub fn undo(&mut self) -> Result<Option<u32>> {
        // Вершина 0 — корень, до первой правки. Отменять нечего, а её
        // `parent` тоже ноль, поэтому смотрим на саму вершину.
        let id = self.current;
        if id == NIL {
            return Ok(None);
        }
        let parent = self.versions[id as usize].parent;
        let record = self.versions[id as usize].record;
        self.current = parent;
        match record {
            // Отмена вставки: вырезать вставленное.
            Record::Insert { anchor, head, .. } => self.tree.cut(anchor, head),
            // Отмена удаления: вернуть вырезанное.
            Record::Delete { anchor, head, text } => {
                let len = head - anchor;
                let start = self.text.len() as u32;
                let piece = self.text[text as usize..text as usize + len as usize].to_vec();
                self.text.extend_from_slice(&piece);
                self.kept.push(start + len);
                let leaf = self.tree.alloc(Node::leaf(start, len));
                self.tree.insert_leaf(anchor, leaf);
            }
        }
        Ok(Some(id))
    }

    /// Повторить отменённую правку.
    pub fn redo(&mut self) -> Result<Option<u32>> {
        let child = self.versions[self.current as usize].child;
        if child == NIL {
            return Ok(None);
        }
        self.current = child;
        let record = self.versions[child as usize].record;
        match record {
            Record::Insert { anchor, head, text } => {
                let len = head - anchor;
                let leaf = self.tree.alloc(Node::leaf(text, len));
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
        }
        Ok(Some(child))
    }

    /// Применить запись и сделать её текущей версией.
    ///
    /// Обе разновидности: иначе при разборе истории удаления терялись бы, а
    /// документ собирался бы неверным.
    pub fn apply(&mut self, record: Record) {
        match record {
            Record::Insert { anchor, head, text } => {
                let len = head - anchor;
                let leaf = self.tree.alloc(Node::leaf(text, len));
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
        }
        self.push(record);
    }

    /// Новая вершина истории от текущей.
    fn push(&mut self, record: Record) {
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.versions.push(Version {
                    record,
                    parent: NIL,
                    child: NIL,
                });
                (self.versions.len() - 1) as u32
            }
        };
        self.versions[id as usize] = Version {
            record,
            parent: self.current,
            child: NIL,
        };
        let parent = self.current;
        self.versions[parent as usize].child = id;
        self.current = id;
    }

    /// Отбросить redo-ветку: вершины в свободный список, текст в хвост.
    fn discard_redo(&mut self) {
        let mut tail = None;
        let mut cursor = self.versions[self.current as usize].child;
        while cursor != NIL {
            let version = self.versions[cursor as usize];
            let off = version.record.text();
            tail = Some(tail.map_or(off, |t: u32| t.min(off)));
            self.free.push(cursor);
            cursor = version.child;
        }
        self.versions[self.current as usize].child = NIL;
        // Хвост не режется ниже текста, вставленного отменой удаления: на
        // него уже ссылаются живые листья.
        let keep = self.kept.iter().copied().max();
        self.kept.clear();
        if let Some(cut) = match (tail, keep) {
            (Some(tail), Some(keep)) => Some(tail.max(keep)),
            (tail, None) => tail,
            (None, _) => None,
        } {
            let cut = cut as usize;
            if cut < self.text.len() {
                self.text.truncate(cut);
            }
        }
    }

    /// Сколько записей истории занято.
    pub fn history_used(&self) -> u32 {
        self.versions.len() as u32 - self.free.len() as u32
    }

    // ── чтение ───────────────────────────────────────────────────────────

    /// Текст диапазона документа.
    fn slice(&self, start: u32, end: u32) -> Vec<u8> {
        #[allow(dead_code)]
        struct Collect<'a> {
            text: &'a [u8],
            out: Vec<u8>,
            base: u32,
        }
        impl crate::tree::Sink for Collect<'_> {
            fn piece(&mut self, start: u32, len: u32) {
                let s = start as usize;
                self.out.extend_from_slice(&self.text[s..s + len as usize]);
            }
        }
        let mut sink = Collect {
            text: &self.text,
            out: Vec::new(),
            base: 0,
        };
        self.tree.read_range_into(start, end, &mut sink);
        sink.out
    }

    /// Весь документ.
    pub fn read(&self) -> Vec<u8> {
        self.slice(0, self.len())
    }

    /// Диапазон документа.
    pub fn read_range(&self, start: u32, end: u32) -> Vec<u8> {
        let start = start.min(self.len());
        let end = end.min(self.len());
        if start >= end {
            return Vec::new();
        }
        self.slice(start, end)
    }

    /// Текст, который был до правок: исходный файл.
    pub fn file_text(&self) -> &[u8] {
        &self.text[..self.file_len as usize]
    }

    /// Записи активной ветки, от корня к текущей вершине.
    pub fn path(&self) -> Vec<Record> {
        let mut out = Vec::new();
        let mut cursor = self.current;
        // Ветка заканчивается на корне, а корень — это ноль, и обход на нём
        // уже остановлен. Отдельной проверки не нужно: у первой правки
        // `parent` тоже ноль, это корень, а не конец ветки.
        while cursor != NIL {
            let version = self.versions[cursor as usize];
            out.push(version.record);
            cursor = version.parent;
        }
        out.reverse();
        out
    }

    // ── снапшот ──────────────────────────────────────────────────────────

    /// Снять снапшот: текст и история уезжают, буфер встаёт на место файла.
    pub fn snapshot(&mut self) {
        let text = self.read();
        let history = crate::history::History::capture(self);
        self.text = text;
        self.file_len = self.text.len() as u32;
        self.tree.collapse_to(0, self.file_len);
        self.kept.clear();
        self.free.clear();
        self.versions.truncate(1);
        self.versions[0].record = Record::Delete {
            anchor: 0,
            head: 0,
            text: 0,
        };
        self.versions[0].parent = NIL;
        self.versions[0].child = NIL;
        self.current = 0;
        self.snapshots += 1;
        if let Some(file) = self.file.clone() {
            self.sink.accept(&file, &self.text, history.as_bytes());
        }
    }

    /// Сколько раз брался снапшот.
    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots
    }

    // ── диагностика ──────────────────────────────────────────────────────

    /// Статистика памяти.
    pub fn memory(&self) -> Memory {
        Memory {
            nodes: self.tree.node_count(),
            free: self.tree.free_len(),
            capacity: self.tree.capacity(),
            leaves: self.tree.leaves,
            node_size: Node::size(),
            text_len: self.text.len(),
            text_capacity: self.text.capacity(),
            versions: self.history_used() as usize,
            version_size: std::mem::size_of::<Version>(),
        }
    }

    /// Высота дерева.
    pub fn height(&self) -> u32 {
        self.tree.height()
    }

    /// Листья по порядку документа.
    pub fn leaves(&self) -> Vec<crate::Leaf> {
        self.tree.leaves_in_order()
    }

    /// Проверка дерева: узлов, высота, зацикливание.
    /// Диагностика дерева: узлов, реальная высота, худший перекос.
    pub fn tree_debug_stats(&self) -> (usize, u32, u32, u32) {
        self.tree.debug_stats()
    }

    pub fn debug_walk(&self) -> (usize, usize, Option<u32>) {
        self.tree.debug_walk()
    }
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("len", &self.tree.len)
            .field("leaves", &self.tree.leaves)
            .field("height", &self.tree.height())
            .finish()
    }
}

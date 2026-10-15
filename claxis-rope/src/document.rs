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
    /// Замена диапазона: что вырезано, что вставлено и где оба текста.
    ///
    /// Одна запись, а не две: замена — одно действие пользователя и один
    /// шаг отмены. Разбирать её на удаление и вставку значило бы платить два
    /// раза за перестройку дерева и путать историю.
    Replace {
        /// Где начинается вырезанный диапазон.
        anchor: u32,
        /// Где кончается заменённый диапазон: это длина вставленного.
        head: u32,
        /// Где в буфере лежит вставленный текст.
        text: u32,
        /// Где в буфере лежит вырезанный текст: он вернётся при отмене.
        removed: u32,
        /// Сколько байт было вырезано.
        removed_len: u32,
    },
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
            Record::Insert { anchor, .. }
            | Record::Delete { anchor, .. }
            | Record::Replace { anchor, .. } => *anchor,
        }
    }

    /// Конец диапазона.
    pub fn head(&self) -> u32 {
        match self {
            Record::Insert { head, .. }
            | Record::Delete { head, .. }
            | Record::Replace { head, .. } => *head,
        }
    }

    /// Смещение текста в буфере.
    pub fn text(&self) -> u32 {
        match self {
            Record::Insert { text, .. } | Record::Delete { text, .. } => *text,
            Record::Replace { text, .. } => *text,
        }
    }

    /// Смещение вырезанного текста, если запись его несёт.
    ///
    /// У вставки и удаления вырезанного текста нет: вставка ничего не
    /// вырезала, а удаление ничего не вставляло.
    pub fn removed(&self) -> Option<(u32, u32)> {
        match self {
            Record::Replace {
                removed,
                removed_len,
                ..
            } => Some((*removed, *removed_len)),
            _ => None,
        }
    }

    /// Вид записи для файла истории.
    pub fn kind(&self) -> Kind {
        match self {
            Record::Insert { .. } => Kind::Insert,
            Record::Delete { .. } => Kind::Delete,
            Record::Replace { .. } => Kind::Replace,
        }
    }
}

/// Вид записи истории.
///
/// Отдельный тип, а не число: что попадёт в файл определяется здесь, и
/// разбор читает то же самое. Перепутать местами нельзя.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Запись вставки.
    Insert,
    /// Запись удаления.
    Delete,
    /// Запись замены.
    Replace,
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
    /// Запись истории ссылается на кусок буфера, которого нет.
    ///
    /// Смещения приходят из блоба, написанного прошлым запуском. Битый блоб —
    /// это ошибка данных, а не повод ронять редактор вместе с текстом.
    HistoryOutOfBounds {
        /// Откуда кусок.
        at: u32,
        /// Сколько байт просили.
        len: u32,
    },
    /// Запись истории противоречива: диапазон вывернут наоборот.
    ///
    /// Всё то же самое, что битый блоб из базы: данные не наши или испорчены.
    InvalidRecordRange {
        /// Откуда диапазон.
        anchor: u32,
        /// Куда диапазон.
        head: u32,
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
            Error::HistoryOutOfBounds { at, len } => {
                messages.history_out_of_bounds(*at, at.saturating_add(*len))
            }
            Error::InvalidRecordRange { anchor, head } => {
                messages.invalid_record_range(*anchor, *head)
            }
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
    sink: Box<dyn SnapshotSink>,
    file: Option<PathBuf>,
    snapshots: u64,
}

impl Document {
    /// Документ с текстом и глубиной истории по умолчанию.
    ///
    /// Без паники и без `Result`: глубина по умолчанию задана самим крейтом и
    /// проверять тут нечего. Провка остаётся у [`Document::with_depth`], где
    /// глубину задаёт пользователь.
    pub fn new(text: impl AsRef<[u8]>) -> Self {
        let text = text.as_ref().to_vec();
        let len = text.len() as u32;
        Self::assemble(text, len, DEFAULT_DEPTH)
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
        let removed = self.slice(anchor, anchor + len);
        self.text.extend_from_slice(&removed);
        let record = Record::Delete {
            anchor,
            head: anchor + len,
            text: start,
        };
        self.apply(record);
        Ok(())
    }

    /// Заменить диапазон: удаление и вставка.
    /// Заменить диапазон на новый текст.
    ///
    /// Одна запись в истории и один проход по дереву: вырезание и вставка
    /// делаются за один раз, а не как две отдельные правки.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<()> {
        self.check(anchor, len)?;
        if len == 0 && data.is_empty() {
            return Ok(());
        }
        self.discard_redo();

        // Вырезанное копируется в хвост буфера: иначе отменять нечем, а второго
        // буфера быть не должно.
        let removed = self.slice(anchor, anchor + len);
        let removed_at = self.text.len() as u32;
        self.text.extend_from_slice(&removed);

        let text = self.text.len() as u32;
        self.text.extend_from_slice(data);

        let record = Record::Replace {
            anchor,
            head: anchor + data.len() as u32,
            text,
            removed: removed_at,
            removed_len: len,
        };
        let new_leaf = self.tree.alloc(Node::leaf(text, data.len() as u32));
        self.tree.replace(anchor, len, new_leaf);
        self.push(record);
        Ok(())
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
                let restored = self.text_at(text, len)?;
                self.restore_at(anchor, restored);
            }
            // Отмена замены: вырезать вставленное и вернуть вырезанное.
            Record::Replace {
                anchor,
                head,
                removed,
                removed_len,
                ..
            } => {
                self.tree.cut(anchor, head);
                if removed_len == 0 {
                    return Ok(Some(id));
                }
                let restored = self.text_at(removed, removed_len)?;
                self.restore_at(anchor, restored);
            }
        }
        Ok(Some(id))
    }

    /// Кусок буфера по смещению из записи истории.
    ///
    /// Смещение приходит из блоба, который писал прошлый запуск: доверять ему
    /// нельзя, а паника из-за битой записи уронила бы редактор вместе с
    /// несохранённым текстом. Не попали — ошибка, документ остаётся целым.
    fn text_at(&self, at: u32, len: u32) -> Result<Vec<u8>> {
        self.text
            .get(at as usize..at as usize + len as usize)
            .map(<[u8]>::to_vec)
            .ok_or(Error::HistoryOutOfBounds { at, len })
    }

    /// Положить кусок в хвост буфера и вставить его на место.
    fn restore_at(&mut self, anchor: u32, restored: Vec<u8>) {
        let len = restored.len() as u32;
        let start = self.text.len() as u32;
        self.text.extend_from_slice(&restored);
        let leaf = self.tree.alloc(Node::leaf(start, len));
        self.tree.insert_leaf(anchor, leaf);
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
            Record::Replace {
                anchor,
                head,
                text,
                removed_len,
                ..
            } => {
                self.tree.cut(anchor, anchor + removed_len);
                let leaf = self.tree.alloc(Node::leaf(text, head - anchor));
                self.tree.insert_leaf(anchor, leaf);
            }
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
            Record::Replace {
                anchor,
                head,
                text,
                removed_len,
                ..
            } => {
                self.tree.cut(anchor, anchor + removed_len);
                let leaf = self.tree.alloc(Node::leaf(text, head - anchor));
                self.tree.insert_leaf(anchor, leaf);
            }
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
        // Резать можно только то, на что больше никто не ссылается. На хвост
        // буфера ссылаются не только отброшенные записи: живые листья дерева и
        // записи истории на активной ветке. Если хоть один из них смотрит
        // дальше, резать нельзя — иначе лист останется читать обрезанный
        // участок и потеряет байты.
        // Резать можно только то, на что больше никто не смотрит. На хвост
        // буфера ссылаются не только отброшенные записи: живые листья дерева и
        // записи истории на активной ветке. Обрезка оставляет начало буфера,
        // поэтому резать можно только за самым концом живых ссылок — иначе
        // лист останется читать обрезанный участок и потеряет байты.
        if tail.is_some() {
            let end = self.live_end() as usize;
            if end < self.text.len() {
                self.text.truncate(end);
            }
        }
    }

    /// Самое позднее место в буфере, до которого дотягиваются живые ссылки.
    ///
    /// Это листья дерева и записи истории, которые ещё можно отменить или
    /// повторить. Всё, что дальше, никому не нужно и только занимает память.
    fn live_end(&self) -> u32 {
        let mut end = 0;
        for leaf in self.tree.leaves_in_order() {
            end = end.max(leaf.start + leaf.len);
        }
        for (id, version) in self.versions.iter().enumerate() {
            // Отброшенная запись ни на что не смотрит, её текст можно резать.
            if self.free.contains(&(id as u32)) {
                continue;
            }
            let record = &version.record;
            end = end.max(record.text() + record.len());
            if let Some((at, len)) = record.removed() {
                end = end.max(at + len);
            }
        }
        end
    }

    /// Сколько записей истории занято.
    pub fn history_used(&self) -> u32 {
        self.versions.len() as u32 - self.free.len() as u32
    }

    // ── чтение ───────────────────────────────────────────────────────────

    /// Текст диапазона документа.
    fn slice(&self, start: u32, end: u32) -> Vec<u8> {
        struct Collect<'a> {
            text: &'a [u8],
            out: Vec<u8>,
        }
        impl crate::tree::Sink for Collect<'_> {
            /// Диапазон листа обрезается по буферу.
            ///
            /// Дерево обещает, что лист лежит в буфере, но обещание даёт код
            /// на сто строчек ниже. Читать с проверкой дешевле, чем чинить
            /// редактор из-за выхода за границу: обрезанный кусок — это
            /// потерянные байты, паника — это упавший редактор.
            fn range(&mut self, start: u32, len: u32) {
                let from = (start as usize).min(self.text.len());
                let to = (start as usize)
                    .saturating_add(len as usize)
                    .min(self.text.len());
                if from < to {
                    self.out.extend_from_slice(&self.text[from..to]);
                }
            }
        }
        let mut sink = Collect {
            text: &self.text,
            out: Vec::new(),
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
        // Основа не длиннее буфера: так её и кладут. Но приходят сюда и
        // восстановленные из базы документы, а обрезать кусок безопаснее, чем
        // упасть.
        self.text
            .get(..self.file_len as usize)
            .unwrap_or(&self.text)
    }

    /// Текст, который эта запись вставила.
    ///
    /// Кто повторяет правку — тот и спрашивает здесь кусок: держать текст ещё
    /// и в записи нельзя.
    pub fn inserted_text(&self, record: &Record) -> Option<&[u8]> {
        let len = record.len();
        if len == 0 {
            return None;
        }
        let at = record.text() as usize;
        self.text.get(at..at + len as usize)
    }

    /// Текст вырезанного куска по записи.
    ///
    /// Для вставки возвращается `None`: у неё своего вырезанного нет.
    pub fn removed_text(&self, record: &Record) -> Option<&[u8]> {
        // Вырезанное лежит в append-буфере записи, а не там, где оно стояло в
        // документе: после отмены на прежнем месте может быть совсем другой текст.
        // Смещения берём те же, что и у самой отмены, иначе карта строк и текст
        // разъедутся. У вставки вырезанного нет вовсе.
        let (at, len) = match record {
            Record::Replace {
                removed,
                removed_len,
                ..
            } => (*removed, *removed_len),
            // Длина берётся через `abs_diff`: у испорченной записи `head`
            // может оказаться меньше `anchor`, и простое вычитание переполнилось
            // бы. `abs_diff` даёт осмысленную длину, а дальше границу проверит
            // `get`.
            Record::Delete { text, .. } => (*text, record.len()),
            Record::Insert { .. } => return None,
        };
        self.text.get(at as usize..at as usize + len as usize)
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

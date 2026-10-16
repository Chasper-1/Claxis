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
///
/// Смещения названы тем, чем они являются: `text_offset` — с какого места общего
/// буфера начинается текст этой записи. Это не текст документа и не позиция в нём.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Record {
    /// Вставка: куда, сколько и где в буфере лежит вставленное.
    Insert {
        /// Где в документе.
        anchor: u32,
        /// Где кончается вставка: это её длина.
        head: u32,
        /// С какого места буфера начинается вставленный текст.
        text_offset: u32,
    },
    /// Удаление: что вырезано и где в буфере это лежит.
    Delete {
        /// Где начинается вырезанный диапазон.
        anchor: u32,
        /// Где кончается вырезанный диапазон.
        head: u32,
        /// С какого места буфера начинается вырезанный текст: он вернётся при
        /// отмене.
        text_offset: u32,
    },
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
        /// С какого места буфера начинается вставленный текст.
        text_offset: u32,
        /// С какого места буфера начинается вырезанный текст: он вернётся при
        /// отмене.
        removed_offset: u32,
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

    /// С какого места буфера начинается текст записи.
    pub fn text_offset(&self) -> u32 {
        match self {
            Record::Insert { text_offset, .. } | Record::Delete { text_offset, .. } => *text_offset,
            Record::Replace { text_offset, .. } => *text_offset,
        }
    }

    /// Смещение вырезанного текста, если запись его несёт.
    ///
    /// У вставки и удаления вырезанного текста нет: вставка ничего не
    /// вырезала, а удаление ничего не вставляло.
    pub fn removed(&self) -> Option<(u32, u32)> {
        match self {
            Record::Replace {
                removed_offset,
                removed_len,
                ..
            } => Some((*removed_offset, *removed_len)),
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

/// Самое позднее место буфера, куда смотрит запись: её собственный текст и
/// вырезанный ею.
fn record_end(record: &Record) -> u32 {
    let mut end = record.text_offset().saturating_add(record.len());
    if let Some((offset, len)) = record.removed() {
        end = end.max(offset.saturating_add(len));
    }
    end
}

/// Вершина истории: запись, родитель, ребёнок и граница буфера.
#[derive(Clone, Copy, Debug)]
struct Version {
    record: Record,
    parent: u32,
    child: u32,
    /// Самое позднее место буфера, куда заглядывает эта вершина или любая
    /// выше неё по родителям.
    ///
    /// Считается один раз при добавлении вершины, а обходом ветки не
    /// пересчитывается: иначе граница обрезки буфера стоила бы столько же,
    /// сколько вся история. Разрез зависит только от записей, и у каждой
    /// вершины он известен сразу.
    live: u32,
}

/// Что пошло не так.
#[derive(Clone, PartialEq, Eq, Debug)]
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
        /// С какого места буфера просили кусок.
        offset: u32,
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
    /// Снимок, который подтянулся из базы для отката дальше, не разобрался.
    ///
    /// Откат на этом заканчивается: продолжать нечего, а молча остановиться
    /// значит выглядеть так, будто отменять больше нечего.
    OlderSnapshotBroken {
        /// Что с блобом.
        reason: String,
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
            Error::HistoryOutOfBounds { offset, len } => {
                messages.history_out_of_bounds(*offset, offset.saturating_add(*len))
            }
            Error::InvalidRecordRange { anchor, head } => {
                messages.invalid_record_range(*anchor, *head)
            }
            Error::OlderSnapshotBroken { reason } => messages.older_snapshot_broken(reason),
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
    /// Ошибка не возвращается: потеря кеша не должна стоить пользователю
    /// работы.
    fn accept(&mut self, path: &std::path::Path, document: &[u8], history: &[u8]);

    /// Блок истории снимка старее того, на котором история в памяти закончилась.
    ///
    /// Вызывается, когда отмена дошла до конца памяти: в ней лежит всего один
    /// снимок, а откатиться надо дальше. Приёмник отдаёт следующий и держит
    /// его в памяти вместо прежнего — так следующая граница не читает диск.
    ///
    /// `None`, когда идти дальше некуда: снимков больше нет.
    fn older(&mut self, path: &std::path::Path) -> Option<Vec<u8>> {
        let _ = path;
        None
    }
}

/// Приёмник, который ничего не делает.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullSink;

impl SnapshotSink for NullSink {
    fn accept(&mut self, _path: &std::path::Path, _text: &[u8], _history: &[u8]) {}
}

/// Документ: текст, история и снапшоты.
pub struct Document {
    /// Байты под деревом: исходный файл в начале, дальше вставки и вырезанное.
    ///
    /// Именно байты, а не текст документа: сам текст собирается обходом дерева,
    /// и путать эти две вещи нельзя. Публичное имя того же — `buffer`.
    bytes: Vec<u8>,
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
    /// Сколько раз документ схлопывался в базу после сохранения.
    compacts: u64,
    /// Сколько раз откат переходил через границу памяти.
    ///
    /// Переход означает, что текст сменился целиком: документ взят из
    /// снимка. Всё, что строилось по тексту по частям — карта строк, например,
    /// — после такого надо собирать заново, а не править дельтой.
    crossings: u64,
}

impl Document {
    /// Документ с текстом и глубиной истории по умолчанию.
    ///
    /// Без паники и без `Result`: глубина по умолчанию задана самим крейтом и
    /// проверять тут нечего. Провка остаётся у [`Document::with_depth`], где
    /// глубину задаёт пользователь.
    pub fn new(bytes: impl AsRef<[u8]>) -> Self {
        let bytes = bytes.as_ref().to_vec();
        let len = bytes.len() as u32;
        Self::assemble(bytes, len, DEFAULT_DEPTH)
    }

    /// Документ с заданной глубиной истории.
    pub fn with_depth(bytes: impl AsRef<[u8]>, depth: u32) -> Result<Self> {
        if depth < MIN_DEPTH {
            return Err(Error::InvalidHistoryDepth { depth });
        }
        let bytes = bytes.as_ref().to_vec();
        let len = bytes.len() as u32;
        Ok(Self::assemble(bytes, len, depth))
    }

    /// Документ из готового буфера: текст уже состоит из файла и хвоста.
    ///
    /// Основание — первые `file_len` байт, остальное хвост: так записи
    /// получают верные смещения, а документ остаётся ровно файлом.
    pub fn from_buffer(bytes: Vec<u8>, file_len: usize) -> Self {
        let file_len = file_len.min(bytes.len());
        let file_len = file_len as u32;
        Self::assemble(bytes, file_len, DEFAULT_DEPTH)
    }

    fn assemble(bytes: Vec<u8>, file_len: u32, depth: u32) -> Self {
        Self {
            tree: Tree::with_leaf(0, file_len),
            bytes,
            file_len,
            versions: vec![Version {
                record: Record::Delete {
                    anchor: 0,
                    head: 0,
                    text_offset: 0,
                },
                parent: NIL,
                child: NIL,
                live: 0,
            }],
            free: Vec::new(),
            current: 0,
            depth,
            sink: Box::new(NullSink),
            file: None,
            snapshots: 0,
            compacts: 0,
            crossings: 0,
        }
    }

    /// Весь текстовый буфер: файл и хвост вставок и вырезанного.
    pub fn buffer(&self) -> &[u8] {
        &self.bytes
    }

    /// Длина текстового буфера.
    pub fn buffer_len(&self) -> usize {
        self.bytes.len()
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
        let text_offset = self.append(data);
        let len = data.len() as u32;
        let record = Record::Insert {
            anchor,
            head: anchor + len,
            text_offset,
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
        let removed = self.slice(anchor, anchor + len);
        let text_offset = self.append(&removed);
        let record = Record::Delete {
            anchor,
            head: anchor + len,
            text_offset,
        };
        self.apply(record);
        Ok(())
    }

    /// Заменить диапазон: удаление и вставка.
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
        let removed_offset = self.append(&removed);
        let text_offset = self.append(data);

        let record = Record::Replace {
            anchor,
            head: anchor + data.len() as u32,
            text_offset,
            removed_offset,
            removed_len: len,
        };
        self.apply_replace(record);
        Ok(())
    }

    /// Положить текст в хвост буфера и сказать, с какого места он лежит.
    ///
    /// Буфер только растёт и переезжать не любит: запись правки хранит
    /// смещение, и оно обязано пережить всё, что случится с деревом.
    fn append(&mut self, data: &[u8]) -> u32 {
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(data);
        offset
    }

    /// Отменить одну правку обратной операцией.
    ///
    /// Отмена не заканчивается там, где кончилась память: дойдя до конца,
    /// документ подтягивает из приёмника снимок старее и продолжает откат с
    /// него. В памяти лежит всего один снимок, поэтому переход за границу
    /// читает диск — но он происходит один раз на снимок, а не на правку.
    pub fn undo(&mut self) -> Result<Option<u32>> {
        // Вершина 0 — корень, до первой правки. Отменять нечего, а её
        // `parent` тоже ноль, поэтому смотрим на саму вершину.
        if self.current == NIL
            && let Some(step) = self.step_over_the_boundary()?
        {
            return Ok(Some(step));
        }
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
            Record::Delete {
                anchor,
                head,
                text_offset,
            } => {
                let len = head - anchor;
                self.restore(anchor, text_offset, len)?;
            }
            // Отмена замены: вырезать вставленное и вернуть вырезанное.
            Record::Replace {
                anchor,
                head,
                removed_offset,
                removed_len,
                ..
            } => {
                self.tree.cut(anchor, head);
                if removed_len == 0 {
                    return Ok(Some(id));
                }
                self.restore(anchor, removed_offset, removed_len)?;
            }
        }
        Ok(Some(id))
    }

    /// Перейти за границу памяти: взять снимок старее и отменить в нём одну
    /// правку.
    ///
    /// Снимок приходит целиком — с текстом и с историей, — поэтому документ
    /// просто восстанавливается целиком. Дальше откат идёт по его истории, а он
    /// в приёмнике остаётся: следующая граница не читает диск заново.
    fn step_over_the_boundary(&mut self) -> Result<Option<u32>> {
        let Some(file) = self.file.clone() else {
            return Ok(None);
        };
        let Some(history) = self.sink.older(&file) else {
            return Ok(None);
        };
        let snapshot = crate::history::History::decode(&history)
            .and_then(|h| h.restore())
            .map_err(|e| Error::OlderSnapshotBroken { reason: e.text() })?;
        let sink = std::mem::replace(&mut self.sink, Box::new(NullSink));
        let file = self.file.clone();
        let crossings = self.crossings;
        *self = snapshot;
        // Приёмник и путь переживают смену документа: они принадлежат файлу,
        // а не состоянию. Без них следующая граница была бы последней.
        self.sink = sink;
        self.file = file;
        self.crossings = crossings + 1;
        self.undo()
    }

    /// Положить копию куска в хвост буфера и вставить её на место.
    ///
    /// Копия, а не ссылка на прежнее место: запись, чей текст возвращается,
    /// после отмены уходит в redo-ветку, а redo-ветку следующая правка
    /// отбрасывает вместе с текстом. Лист, смотрящий на этот текст, пережил бы
    /// обрезку буфера и прочитал бы чужое.
    fn restore(&mut self, anchor: u32, from: u32, len: u32) -> Result<()> {
        let end = from
            .checked_add(len)
            .ok_or(Error::HistoryOutOfBounds { offset: from, len })?;
        let restored = self
            .bytes
            .get(from as usize..end as usize)
            .ok_or(Error::HistoryOutOfBounds { offset: from, len })?
            .to_vec();
        let start = self.append(&restored);
        let leaf = self.tree.alloc(Node::leaf(start, len));
        self.tree.insert_leaf(anchor, leaf);
        Ok(())
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
            Record::Insert {
                anchor,
                head,
                text_offset,
            } => {
                let leaf = self.tree.alloc(Node::leaf(text_offset, head - anchor));
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
            Record::Replace {
                anchor,
                head,
                text_offset,
                removed_len,
                ..
            } => {
                self.tree.cut(anchor, anchor + removed_len);
                let leaf = self.tree.alloc(Node::leaf(text_offset, head - anchor));
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
            Record::Insert {
                anchor,
                head,
                text_offset,
            } => {
                let leaf = self.tree.alloc(Node::leaf(text_offset, head - anchor));
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
            Record::Replace {
                anchor,
                head,
                text_offset,
                removed_len,
                ..
            } => {
                // Вырезание и вставка одним проходом: так дерево перестраивается
                // один раз, а не два. Разбор истории идёт тем же путём.
                let leaf = self.tree.alloc(Node::leaf(text_offset, head - anchor));
                self.tree.replace(anchor, removed_len, leaf);
            }
        }
        self.push(record);
    }

    /// Применить замену изнутри: дерево трогается здесь, история — в записи.
    ///
    /// Отдельный метод не для красоты: `apply` разбирает записи, пришедшие из
    /// базы, где у замены вырезанное лежит по своему смещению. Здесь же
    /// вырезанное уже в буфере, и путь другой — одним проходом.
    fn apply_replace(&mut self, record: Record) {
        let Record::Replace {
            anchor,
            head,
            text_offset,
            removed_len,
            ..
        } = record
        else {
            return;
        };
        let leaf = self.tree.alloc(Node::leaf(text_offset, head - anchor));
        self.tree.replace(anchor, removed_len, leaf);
        self.push(record);
    }

    /// Новая вершина истории от текущей.
    fn push(&mut self, record: Record) {
        let live = self.versions[self.current as usize]
            .live
            .max(record_end(&record));
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.versions.push(Version {
                    record,
                    parent: NIL,
                    child: NIL,
                    live,
                });
                (self.versions.len() - 1) as u32
            }
        };
        self.versions[id as usize] = Version {
            record,
            parent: self.current,
            child: NIL,
            live,
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
            let off = version.record.text_offset();
            tail = Some(tail.map_or(off, |t: u32| t.min(off)));
            self.free.push(cursor);
            cursor = version.child;
        }
        self.versions[self.current as usize].child = NIL;
        // Резать можно только то, на что больше никто не смотрит. На хвост
        // буфера ссылаются не только отброшенные записи: живые листья дерева и
        // записи истории на активной ветке. Обрезка оставляет начало буфера,
        // поэтому резать можно только за самым концом живых ссылок — иначе
        // лист останется читать обрезанный участок и потеряет байты.
        if tail.is_some() {
            let end = self.live_end() as usize;
            if end < self.bytes.len() {
                self.bytes.truncate(end);
            }
        }
    }

    /// Самое позднее место в буфере, до которого дотягиваются живые ссылки.
    ///
    /// Три источника. Исходный файл: его лист один и он вечен. Записи живой
    /// ветки: их текст лежит в буфере, хотя в дереве его уже нет, — без него
    /// отмена удаления нечем вернуть. Листья дерева: вставленный текст и те
    /// копии, которые положила отмена.
    ///
    /// Записи ветки берутся готовым числом у вершины, а не обходом: иначе
    /// граница обрезки стоила бы столько же, сколько вся история. Листья
    /// смотрятся обходом дерева — он идёт только при отброшенной ветке, то
    /// есть не на каждое нажатие, а на замену ветки на другую.
    fn live_end(&self) -> u32 {
        self.file_len
            .max(self.versions[self.current as usize].live)
            .max(self.tree.leaf_end())
    }

    /// Сколько записей истории занято.
    pub fn history_used(&self) -> u32 {
        self.versions.len() as u32 - self.free.len() as u32
    }

    // ── чтение ───────────────────────────────────────────────────────────

    /// Текст диапазона документа.
    fn slice(&self, start: u32, end: u32) -> Vec<u8> {
        struct Collect<'a> {
            bytes: &'a [u8],
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
                let from = (start as usize).min(self.bytes.len());
                let to = (start as usize)
                    .saturating_add(len as usize)
                    .min(self.bytes.len());
                if from < to {
                    self.out.extend_from_slice(&self.bytes[from..to]);
                }
            }
        }
        let mut sink = Collect {
            bytes: &self.bytes,
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

    /// Основа документа: текст файла, каким он был до правок.
    ///
    /// Не весь документ: правки лежат в хвосте буфера, и собирает их дерево.
    pub fn base(&self) -> &[u8] {
        // Основа не длиннее буфера: так её и кладут. Но приходят сюда и
        // восстановленные из базы документы, а обрезать кусок безопаснее, чем
        // упасть.
        self.bytes
            .get(..self.file_len as usize)
            .unwrap_or(&self.bytes)
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
        let offset = record.text_offset() as usize;
        self.bytes.get(offset..offset + len as usize)
    }

    /// Текст вырезанного куска по записи.
    ///
    /// Для вставки возвращается `None`: у неё своего вырезанного нет.
    pub fn removed_text(&self, record: &Record) -> Option<&[u8]> {
        // Вырезанное лежит в append-буфере записи, а не там, где оно стояло в
        // документе: после отмены на прежнем месте может быть совсем другой текст.
        // Смещения берём те же, что и у самой отмены, иначе карта строк и текст
        // разъедутся. У вставки вырезанного нет вовсе.
        let (offset, len) = match record {
            Record::Replace {
                removed_offset,
                removed_len,
                ..
            } => (*removed_offset, *removed_len),
            // Длина берётся через `abs_diff`: у испорченной записи `head`
            // может оказаться меньше `anchor`, и простое вычитание переполнилось
            // бы. `abs_diff` даёт осмысленную длину, а дальше границу проверит
            // `get`.
            Record::Delete { text_offset, .. } => (*text_offset, record.len()),
            Record::Insert { .. } => return None,
        };
        self.bytes
            .get(offset as usize..offset as usize + len as usize)
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

    /// Снять снапшот: текст и вся история уезжают, память встаёт на место снимка.
    ///
    /// В памяти остаётся **один** снимок — тот, что снят последним, — и он же
    /// лежит в кольце на диске. Откат доходит до него по памяти, а уйти дальше
    /// приёмник отдаёт из базы следующий: в базе он уже есть, его читают, а не
    /// забирают.
    pub fn snapshot(&mut self) {
        let document = self.read();
        let history = crate::history::History::capture(self);
        self.snapshots += 1;
        let Some(file) = self.file.clone() else {
            // Снимку некуда уйти, значит он не сделан. Схлопывать память
            // тогда нельзя: история нигде не лежит и пропала бы вместе с
            // возможностью отменить.
            return;
        };
        self.sink.accept(&file, &document, history.as_bytes());
        self.compact();
    }

    /// Схлопнуть документ в одну базу и обрезать историю в памяти.
    ///
    /// Память не должна расти вместе с правками, поэтому после снимка
    /// история обрезается, а то, что было до неё, лежит в снимке.
    fn compact(&mut self) {
        let document = self.read();
        self.bytes = document;
        self.file_len = self.bytes.len() as u32;
        self.tree.collapse_to(0, self.file_len);
        self.free.clear();
        self.versions.truncate(1);
        self.versions[0].record = Record::Delete {
            anchor: 0,
            head: 0,
            text_offset: 0,
        };
        self.versions[0].parent = NIL;
        self.versions[0].child = NIL;
        self.current = 0;
        self.compacts += 1;
    }

    /// Сколько раз брался снапшот.
    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots
    }

    /// Сколько раз документ схлопывался после сохранения.
    pub fn compacts_taken(&self) -> u64 {
        self.compacts
    }

    /// Сколько раз откат переходил через границу памяти.
    ///
    /// Число растёт, когда текст сменился целиком: значит, всё построенное по
    /// тексту по частям после этого уже не годится.
    pub fn crossings(&self) -> u64 {
        self.crossings
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
            text_len: self.bytes.len(),
            text_capacity: self.bytes.capacity(),
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

#[cfg(test)]
mod tests {
    //! Проверки документа: правки, отмены, снапшоты и баланс дерева.
    //!
    //! Главная проверка — сверка с эталоном на длинной случайной
    //! последовательности правок с перемешиванием отмен и повторов. Эталон —
    //! обычная строка, и любое расхождение означает битый документ.

    use super::*;

    /// Кольцо снимков в памяти: пишется всё, отдаётся по убыванию.
    ///
    /// Так же устроено кольцо на диске, только без второй копии под рукой.
    #[derive(Debug, Default)]
    struct Ring {
        written: Vec<(Vec<u8>, Vec<u8>)>,
        served: usize,
    }

    /// Приёмник, умеющий делиться кольцом с тестом: приёмник уезжает внутрь
    /// документа, а кольцо нужно и там, и снаружи.
    #[derive(Debug, Clone)]
    struct Share(std::rc::Rc<std::cell::RefCell<Ring>>);

    impl SnapshotSink for Share {
        fn accept(&mut self, path: &std::path::Path, text: &[u8], history: &[u8]) {
            self.0
                .borrow_mut()
                .written
                .push((text.to_vec(), history.to_vec()));
            let _ = path;
        }

        fn older(&mut self, _path: &std::path::Path) -> Option<Vec<u8>> {
            let ring = &mut *self.0.borrow_mut();
            // Снимки лежат по убыванию: последний, предпоследний, и так
            // далее. В базе они остаются — отсюда просто читаются.
            let at = ring.written.len().checked_sub(ring.served + 1)?;
            ring.served += 1;
            ring.written.get(at).map(|(_, history)| history.clone())
        }
    }

    #[test]
    fn undo_crosses_the_memory_boundary_into_the_ring() {
        // Три снимка, между ними по одной правке. В памяти после каждого
        // снимка пусто, но откат идёт дальше по кольцу — и не должен кончаться
        // на границе.
        let ring = std::rc::Rc::new(std::cell::RefCell::new(Ring::default()));
        let mut doc = Document::new(b"abc");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Share(std::rc::Rc::clone(&ring))),
        );
        for _ in 0..3 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
        }
        assert_eq!(ring.borrow().written.len(), 3, "все три в кольце");
        assert_eq!(doc.path().len(), 0, "в памяти пусто");

        doc.undo().unwrap().expect("откат за границу памяти");
        assert_eq!(doc.read(), b"abcxx");
        doc.undo().unwrap().expect("и следующий снимок");
        assert_eq!(doc.read(), b"abcx");
        doc.undo().unwrap().expect("и третий");
        assert_eq!(doc.read(), b"abc");
        assert!(doc.undo().unwrap().is_none(), "дальше снимков нет");
    }

    #[test]
    fn an_undo_that_crosses_the_boundary_stays_in_memory() {
        // После перехода через границу отказ должен продолжать работать
        // в том же сеансе: границ сколько угодно, и каждая не конец света.
        let ring = std::rc::Rc::new(std::cell::RefCell::new(Ring::default()));
        let mut doc = Document::new(b"");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Share(std::rc::Rc::clone(&ring))),
        );
        for i in 0..4u32 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
            assert_eq!(doc.read(), "x".repeat(i as usize + 1).into_bytes());
        }

        for i in (0..4u32).rev() {
            doc.undo().unwrap().expect("откат идёт");
            assert_eq!(doc.read(), "x".repeat(i as usize).into_bytes());
        }
        assert!(doc.undo().unwrap().is_none());
    }

    use crate::history;

    fn d(text: &str) -> Document {
        Document::new(text.as_bytes())
    }

    /// Генератор чисел: детерминированный, чтобы падение воспроизводилось.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                return 0;
            }
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    /// Правка в эталоне: что вставлено и что было там до неё.
    enum Op {
        Insert {
            pos: usize,
            text: Vec<u8>,
        },
        Delete {
            pos: usize,
            removed: Vec<u8>,
        },
        Replace {
            pos: usize,
            removed: Vec<u8>,
            text: Vec<u8>,
        },
    }

    #[test]
    fn insert_splits_and_reads_back() {
        let mut doc = d("abcdefghij");
        doc.insert(5, b"XXXXX").unwrap();
        assert_eq!(doc.read(), b"abcdeXXXXXfghij");
    }

    #[test]
    fn delete_cuts_middle() {
        let mut doc = d("The quick brown fox");
        doc.delete(4, 6).unwrap();
        assert_eq!(doc.read(), b"The brown fox");
    }

    #[test]
    fn replace_is_delete_then_insert() {
        let mut doc = d("The quick brown fox");
        doc.replace(4, 5, b"slow").unwrap();
        assert_eq!(doc.read(), b"The slow brown fox");
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        assert_eq!(doc.read(), b"abcXY");

        doc.undo().unwrap().expect("есть что отменять");
        assert_eq!(doc.read(), b"abc");
        assert!(doc.undo().unwrap().is_none(), "дальше нечего");

        doc.redo().unwrap().expect("есть что повторять");
        assert_eq!(doc.read(), b"abcXY");
        assert!(doc.redo().unwrap().is_none());
    }

    #[test]
    fn undo_of_delete_puts_the_text_back() {
        // Главное: удалённый текст должен вернуться целиком и на своё место.
        let mut doc = d("The quick brown fox");
        doc.delete(4, 5).unwrap();
        assert_eq!(doc.read(), b"The  brown fox");
        doc.undo().unwrap().expect("удаление отменяемо");
        assert_eq!(doc.read(), b"The quick brown fox");
    }

    #[test]
    fn undo_of_a_wide_delete_puts_everything_back() {
        // Удаление через границы листьев — самый частый случай потери текста.
        let mut doc = d("aaa bbb ccc ddd");
        for i in 0..20 {
            doc.insert(i as u32, b"xyz").unwrap();
        }
        let before = doc.read();
        doc.delete(5, 20).unwrap();
        assert_ne!(doc.read(), before);
        doc.undo().unwrap().expect("отменяемо");
        assert_eq!(doc.read(), before, "широкое удаление вернулось не целиком");
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo_branch() {
        let mut doc = d("");
        doc.insert(0, b"A").unwrap();
        doc.insert(1, b"B").unwrap();
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.read(), b"");

        doc.insert(0, b"X").unwrap();
        assert_eq!(doc.read(), b"X");
        assert!(doc.redo().unwrap().is_none(), "ветка отброшена");
        assert_eq!(doc.read(), b"X");
    }

    #[test]
    fn repeated_undo_redo_keeps_the_document_intact() {
        let mut doc = d("base");
        for i in 0..10 {
            doc.insert(i as u32, b"ab").unwrap();
        }
        for _ in 0..10 {
            doc.delete(0, 1).unwrap();
        }
        let end = doc.read();
        // Двадцать правок: десять вставок и десять удалений. Отменить надо все.
        for _ in 0..20 {
            doc.undo().unwrap().expect("отменяемо");
        }
        assert_eq!(doc.read(), b"base");
        assert!(doc.undo().unwrap().is_none());
        for _ in 0..20 {
            doc.redo().unwrap().expect("повторяемо");
        }
        assert_eq!(doc.read(), end);
        assert!(doc.redo().unwrap().is_none());
    }

    #[test]
    fn out_of_range_is_an_error_not_a_panic() {
        let mut doc = d("abc");
        assert!(doc.delete(2, 5).is_err());
        assert!(doc.insert(99, b"x").is_err());
        assert_eq!(doc.read(), b"abc");
    }

    #[test]
    fn the_leaf_counter_tells_the_truth() {
        // Счётчик листьев живёт в дереве, а на него смотрят диагностика и
        // выделение памяти. Разъехаться он может незаметно: листья появляются и
        // исчезают в полудюжине мест, и забыть уменьшить счётчик в одном из
        // них ничего не ломает, кроме самого счётчика. Поэтому он и сверяется
        // здесь с настоящим обходом на смеси правок.
        let mut doc = d("0123456789");
        let mut rng = Rng(7);
        for _ in 0..3000 {
            let len = doc.len();
            let pos = rng.below(len.max(1) as usize) as u32;
            if len > 4 && rng.below(3) == 0 {
                let _ = doc.delete(pos.min(len - 2), 1 + rng.below(4) as u32);
            } else if rng.below(5) == 0 {
                let _ = doc.replace(pos.min(len.saturating_sub(2)), 2, b"q");
            } else {
                doc.insert(pos, b"xy").unwrap();
            }
            let counted = doc.memory().leaves;
            let real = doc.leaves().len();
            assert_eq!(counted, real, "счётчик листьев разошёлся с деревом");
        }
    }

    #[test]
    fn typing_into_the_middle_does_not_double_the_leaves() {
        // Каждое нажатие разрезает лист, и если склейки нет, на нажатие
        // остаётся два листа: набор в середину файла раздувает дерево
        // линейно. Со смежными диапазонами склейка идёт в `concat`, поэтому
        // нажатие подряд добавляет ровно один лист — столько же, сколько
        // вставлено байт, и меньше быть не может: текст разбросан по буферу.
        let mut doc = Document::from_buffer(vec![b'x'; 64 * 1024], 64 * 1024);
        for _ in 0..1000 {
            doc.insert(32_000, b"y").unwrap();
        }
        // Первое нажатие разрезает лист на три — голова, вставка и хвост, — а
        // каждое следующее добавляет один: вставленный тект примыкает к
        // предыдущему в буфере и склеивается с ним.
        assert_eq!(
            doc.leaves().len(),
            1002,
            "на нажатие должен оставаться один лист"
        );
    }

    #[test]
    fn undoing_everything_puts_the_leaves_back_together() {
        // Отмена вставки вырезает вставленное, и оставшиеся куски листа
        // снова смыкаются в буфере. Дерево после полного отката обязано
        // вернуться к одному листу: иначе отмена не «схлопывает» документ, а
        // оставляет после себя мусор, который растёт с каждой серией правок.
        let mut doc = d("0123456789");
        for _ in 0..200 {
            doc.insert(0, b"xy").unwrap();
        }
        assert!(doc.leaves().len() > 1, "правки должны были порезать лист");
        while doc.undo().unwrap().is_some() {}
        assert_eq!(doc.read(), b"0123456789");
        assert_eq!(doc.leaves().len(), 1, "после отката всё должно слипться");
    }

    #[test]
    fn merging_leaves_keeps_the_text_and_the_history() {
        // Склейка не должна ломать ни текст, ни отмену: запись правки живёт в
        // байтах и границ листьев не знает, но проверить это надо на живом
        // документе, где склейка действительно происходит.
        let mut doc = d("0123456789");
        doc.insert(5, b"ab").unwrap();
        doc.delete(0, 3).unwrap();
        doc.replace(1, 2, b"ZZ").unwrap();
        let end = doc.read();
        let leaves_after_edits = doc.leaves().len();

        for _ in 0..3 {
            doc.undo().unwrap().expect("отменяемо");
        }
        assert_eq!(doc.read(), b"0123456789");
        for _ in 0..3 {
            doc.redo().unwrap().expect("повторяемо");
        }
        assert_eq!(doc.read(), end);
        assert_eq!(
            doc.leaves().len(),
            leaves_after_edits,
            "повтор сломал листья"
        );

        // Полный откат после серии правок обязан вернуть исходный текст байт
        // в байт: буфер мог порезаться при отброшенной ветке.
        while doc.undo().unwrap().is_some() {}
        assert_eq!(doc.read(), b"0123456789");
    }

    #[test]
    fn history_depth_has_no_ceiling() {
        // Потолка нет: история растёт вместе с правками, память добирается сама.
        // Отвергается только ноль — с ним снимался бы снапшот после каждой
        // правки.
        assert!(Document::with_depth(b"x", 0).is_err());
        assert!(Document::with_depth(b"x", 1).is_ok());
        assert!(Document::with_depth(b"x", u32::MAX).is_ok());

        // И правок может быть больше, чем любое разумное число.
        let mut doc = Document::with_depth(b"x", u32::MAX / 2).unwrap();
        for _ in 0..50_000u32 {
            doc.insert(0, b"y").unwrap();
        }
        assert_eq!(doc.path().len(), 50_000, "история росла вместе с правками");
        assert_eq!(doc.snapshots_taken(), 0, "снапшот брать было не пора");
    }

    #[test]
    fn a_snapshot_carries_text_and_history() {
        // Снапшот уносит состояние целиком: и текст, и историю. Без истории
        // документ верный, а отменять нечего.
        let (text, history) = {
            let mut doc = d("abc");
            doc.set_sink(std::path::PathBuf::from("/tmp/d.txt"), Box::new(NullSink));
            doc.insert(3, b"XY").unwrap();
            doc.delete(0, 1).unwrap();
            // История снимается до снапшота: снимок её уводит, это его смысл.
            let history = history::History::capture(&doc);
            let text = doc.read();
            doc.snapshot();
            assert_eq!(doc.read(), text, "снапшот текст не меняет");
            (text, history)
        };
        assert_eq!(text, b"bcXY");

        let mut back = history.restore().expect("снапшот разбирается");
        assert_eq!(back.read(), text);
        back.undo().unwrap().expect("история восстановлена");
        assert_eq!(back.read(), b"abcXY");
    }

    #[test]
    fn a_snapshot_keeps_only_the_newest_edit_in_memory() {
        // Снапшот схлопывает память: всё, что было до него, уезжает в кольцо,
        // а в памяти остаётся ровно один снимок. Откат доходит до него без
        // диска, дальше — приёмник отдаёт следующий из базы.
        let mut doc = Document::with_depth(b"abc", 4).unwrap();
        doc.set_sink(std::path::PathBuf::from("/tmp/d.txt"), Box::new(NullSink));
        for i in 0..100 {
            doc.insert(i as u32, b"x").unwrap();
        }
        assert_eq!(doc.path().len(), 100, "история росла вместе с правками");

        doc.snapshot();
        assert_eq!(doc.snapshots_taken(), 1, "по прямой просьбе снимается");
        assert_eq!(doc.path().len(), 0, "в памяти один снимок и ничего");
        assert_eq!(doc.read().len(), 103, "текст на месте");
    }

    #[test]
    fn compacting_cuts_the_history_on_purpose() {
        // Схлопывание — отдельная операция после сохранения файла. Вот тут
        // история обрезается, и это осознанно.
        let mut doc = Document::new(b"abc");
        doc.insert(3, b"XY").unwrap();
        assert_eq!(doc.path().len(), 1);

        doc.compact();
        assert_eq!(doc.compacts_taken(), 1);
        assert_eq!(doc.path().len(), 0, "после схлопывания история пуста");
        assert_eq!(doc.read(), b"abcXY", "текст при этом цел");
        assert!(doc.undo().unwrap().is_none(), "отменять больше нечего");

        // И документ продолжает работать.
        doc.insert(5, b"Z").unwrap();
        assert_eq!(doc.read(), b"abcXYZ");
    }

    #[test]
    fn a_snapshot_leaves_undo_working_after_it() {
        // Главное про память: отмена после снимка живая. Дальше по границу
        // уходит приёмник, а не обрыв цепочки.
        let mut doc = Document::new(b"abc");
        doc.set_sink(std::path::PathBuf::from("/tmp/d.txt"), Box::new(NullSink));
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();

        doc.insert(5, b"Z").unwrap();
        assert_eq!(doc.read(), b"abcXYZ");
        doc.undo().unwrap().expect("правка после снимка отменяется");
        assert_eq!(doc.read(), b"abcXY");
    }

    #[test]
    fn the_newest_snapshot_replaces_the_previous_one_in_memory() {
        // В памяти снимок один: новый вытесняет старый, но старый остаётся в
        // кольце. Откат за границу идёт по ним в порядке убывания.
        let mut doc = Document::new(b"abc");
        doc.set_sink(std::path::PathBuf::from("/tmp/d.txt"), Box::new(NullSink));
        for _ in 0..5 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
        }
        assert_eq!(doc.snapshots_taken(), 5);
        assert_eq!(doc.path().len(), 0, "в памяти один снимок, не пять");
        assert_eq!(doc.read(), b"abcxxxxx");
    }

    #[test]
    fn reading_is_consistent_after_many_edits() {
        let mut doc = d("");
        for i in 0..300u32 {
            doc.insert(doc.len(), b"line\n").unwrap();
            if i % 3 == 0 && doc.len() > 10 {
                doc.delete(0, 5).unwrap();
            }
        }
        let full = doc.read();
        assert_eq!(doc.read_range(0, doc.len()), full);
        assert_eq!(doc.read_range(5, 5), Vec::<u8>::new());
        let mid = doc.read_range(2, 10);
        assert_eq!(mid, full[2..10].to_vec());
    }

    #[test]
    fn the_path_is_the_active_branch_in_order() {
        let mut doc = d("");
        doc.insert(0, b"A").unwrap();
        doc.insert(1, b"B").unwrap();
        doc.undo().unwrap();
        doc.insert(1, b"C").unwrap();

        let path = doc.path();
        assert_eq!(path.len(), 2);
        assert!(matches!(path[0], Record::Insert { head: 1, .. }));
        assert!(matches!(path[1], Record::Insert { head: 2, .. }));
        assert_eq!(doc.read(), b"AC");
    }

    #[test]
    fn against_a_reference_string_with_undo_redo_mix() {
        // Эталон — обычная строка. Случайная последовательность правок, отмен и
        // повторов: любое расхождение означает битый документ.
        for seed in 0..8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(7));
            let mut doc = Document::new(b"seed");
            let mut text = String::from("seed");
            let mut steps: Vec<Op> = Vec::new();
            let mut cursor = 0usize;

            for step in 0..1200 {
                let roll = rng.below(12);
                if roll < 4 && !text.is_empty() {
                    let pos = rng.below(text.len());
                    let len = rng.below((text.len() - pos).min(40)) + 1;
                    let removed = text.as_bytes()[pos..pos + len].to_vec();
                    doc.delete(pos as u32, len as u32).unwrap();
                    text.replace_range(pos..pos + len, "");
                    steps.truncate(cursor);
                    steps.push(Op::Delete { pos, removed });
                    cursor = steps.len();
                } else if roll < 7 {
                    let pos = rng.below(text.len() + 1);
                    let typed: String = (0..rng.below(30) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    doc.insert(pos as u32, typed.as_bytes()).unwrap();
                    text.insert_str(pos, &typed);
                    steps.truncate(cursor);
                    steps.push(Op::Insert {
                        pos,
                        text: typed.into_bytes(),
                    });
                    cursor = steps.len();
                } else if roll < 9 {
                    // Замена: самое интересное, потому что это одна запись,
                    // которая несёт и вставленное, и вырезанное.
                    let pos = rng.below(text.len() + 1);
                    let len = rng.below((text.len() - pos).min(40));
                    let removed = text.as_bytes()[pos..pos + len].to_vec();
                    let typed: String = (0..rng.below(20) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    doc.replace(pos as u32, len as u32, typed.as_bytes())
                        .unwrap();
                    text.replace_range(pos..pos + len, &typed);
                    steps.truncate(cursor);
                    steps.push(Op::Replace {
                        pos,
                        removed,
                        text: typed.into_bytes(),
                    });
                    cursor = steps.len();
                } else if roll == 9 && cursor > 0 {
                    cursor -= 1;
                    doc.undo()
                        .unwrap()
                        .expect("история должна была быть непустой");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => text.replace_range(*pos..*pos + t.len(), ""),
                        Op::Delete { pos, removed } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(removed))
                        }
                        Op::Replace {
                            pos,
                            removed,
                            text: t,
                        } => {
                            // Отмена замены: убрать вставленное, вернуть
                            // вырезанное.
                            text.replace_range(*pos..*pos + t.len(), "");
                            let back = String::from_utf8_lossy(removed).to_string();
                            text.insert_str(*pos, &back);
                        }
                    }
                } else if cursor < steps.len() {
                    doc.redo()
                        .unwrap()
                        .expect("повтор должен был быть доступен");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(t))
                        }
                        Op::Delete { pos, removed } => {
                            text.replace_range(*pos..*pos + removed.len(), "")
                        }
                        Op::Replace {
                            pos,
                            removed,
                            text: t,
                        } => {
                            let new = String::from_utf8_lossy(t).to_string();
                            text.replace_range(*pos..*pos + removed.len(), &new);
                        }
                    }
                    cursor += 1;
                }

                assert_eq!(
                    doc.read(),
                    text.as_bytes(),
                    "seed {seed} step {step}: текст разошёлся"
                );
                assert_eq!(doc.len() as usize, text.len(), "seed {seed} step {step}");
                assert_eq!(doc.path().len(), cursor, "seed {seed} step {step}");
            }
        }
    }

    #[test]
    fn the_tree_stays_balanced_under_replaces_and_undos() {
        // Тот же баланс, но под заменами и отменами: это самый дорогой путь
        // дерева, и рассинхрон высот там виден не сразу, а потом всплывает
        // медленным спуском по документу.
        // Текст латиницей: позиции в эталоне байтовые, и резать строку посреди
        // многобайтового символа нельзя — тест проверяет дерево, а не UTF-8.
        let mut doc = Document::new("start\n".as_bytes());
        let mut rng = Rng(0x5DEECE66D);
        let mut reference = String::from("start\n");
        // Последняя правка была заменой: что вставили и что там было.
        // Отменять умеем только её, поэтому и отменяем только её.
        let mut last_replace: Option<(usize, usize, String)> = None;
        const INSERTED: &str = "replaced\n";

        for _ in 0..1500 {
            match rng.below(5) {
                0 if last_replace.is_some() => {
                    let (pos, _len, before) = last_replace.take().expect("замена была");
                    let after_redo = reference.clone();
                    doc.undo().unwrap().expect("отменять есть что");
                    // Отмена замены: убрать вставленное и вернуть вырезанное.
                    reference.replace_range(pos..pos + INSERTED.len(), &before);
                    assert_eq!(
                        doc.read(),
                        reference.as_bytes(),
                        "отмена замены на {pos} вернула не то: было {:?}, стало {:?}",
                        after_redo,
                        before
                    );
                    doc.redo().unwrap().expect("повторять есть что");
                    // Повтор вернул документ к тому, что было: эталон тоже.
                    reference = after_redo;
                    assert_eq!(doc.read(), reference.as_bytes());
                }
                1 => {
                    let pos = rng.below(reference.len() + 1);
                    let len = rng.below((reference.len() - pos).min(20));
                    let before = reference[pos..pos + len].to_string();
                    doc.replace(pos as u32, len as u32, INSERTED.as_bytes())
                        .unwrap();
                    reference.replace_range(pos..pos + len, INSERTED);
                    last_replace = Some((pos, len, before));
                }
                2 if !reference.is_empty() => {
                    let pos = rng.below(reference.len());
                    let len = 1 + rng.below((reference.len() - pos).min(10));
                    last_replace = None;
                    doc.delete(pos as u32, len as u32).unwrap();
                    reference.replace_range(pos..pos + len, "");
                }
                _ => {
                    let pos = rng.below(reference.len() + 1);
                    last_replace = None;
                    doc.insert(pos as u32, "inserted\n".as_bytes()).unwrap();
                    reference.insert_str(pos, "inserted\n");
                }
            }
            assert_eq!(doc.read(), reference.as_bytes());
        }

        let (_, height, skew, mismatch) = doc.tree_debug_stats();
        assert_eq!(mismatch, 0, "высота в узле разошлась с настоящей");
        assert!(skew <= 1, "перекос {skew} после замен и отмен");
        assert_eq!(height, doc.height(), "высота в корне не совпадает");
    }

    #[test]
    fn tree_stays_balanced_under_churn() {
        // Баланс держится при любом порядке правок: иначе спуск перестаёт быть
        // логарифмическим.
        let mut doc = Document::new(b"a");
        let mut rng = Rng(99);
        for _ in 0..2000 {
            let pos = rng.below(doc.len() as usize + 1) as u32;
            if rng.below(2) == 0 && !doc.read().is_empty() {
                let len = rng.below(doc.len() as usize) + 1;
                let _ = doc.delete(pos.min(doc.len() - 1), len as u32);
            } else {
                doc.insert(pos, b"xy").unwrap();
            }
        }
        let (nodes, height, worst_skew, height_mismatch) = doc.tree_debug_stats();
        let leaves = doc.leaves().len();
        if leaves > 0 {
            // Сплошное дерево из n листьев всегда ровно 2n − 1 узлов.
            assert_eq!(nodes, leaves * 2 - 1, "дерево не сплошное");
        }
        assert_eq!(
            height_mismatch, 0,
            "высота в узле разошлась с настоящей: баланс считается не по тому"
        );
        assert!(worst_skew <= 1, "перекос {worst_skew} при {nodes} узлах");
        assert_eq!(height, doc.height(), "высота в корне не совпадает");
        // Высота AVL: не больше примерно 1.44 от log2(листьев) плюс запас.
        let bound = (leaves.max(2) as f64).log2() * 1.5 + 2.0;
        assert!(
            height as f64 <= bound,
            "высота {height} при {leaves} листьях превышает {bound:.1}"
        );
    }

    #[test]
    fn memory_does_not_run_away() {
        let mut doc = Document::new(b"");
        for i in 0..8000u32 {
            doc.insert(i % (doc.len() + 1), b"xyz").unwrap();
        }
        let m = doc.memory();
        eprintln!(
            "ROPE 8000 правок | листьев {:>5} | узлов {:>5}/{} | вершин {:>5} | текст {:>6}/{} | ПУЛ {:>7} | вершины {:>7} | всего {:>7}",
            m.leaves,
            m.nodes,
            m.capacity,
            m.versions,
            m.text_len,
            m.text_capacity,
            m.tree_bytes(),
            m.versions * m.version_size,
            m.tree_bytes() + m.text_capacity + m.versions * m.version_size,
        );
        assert_eq!(doc.read().len(), 24_000);
        assert_eq!(Document::new(b"").history_depth(), DEFAULT_DEPTH);
    }

    #[test]
    fn undo_and_discard_keep_the_buffer_intact() {
        // Последовательность найдена сжатием падения: отмена удаления
        // возвращает текст в конец буфера, отмена замены его вырезает, а
        // следующая правка отбрасывает redo-ветку. Обрезка резала буфер по
        // отброшенным записям и оставляла лист читать участок, которого уже
        // нет.
        let mut doc = Document::new(b"");
        doc.replace(0, 0, b"xy\n").unwrap();
        doc.replace(3, 0, b"xy\n").unwrap();
        doc.delete(1, 4).unwrap();
        doc.undo().unwrap().expect("отменять есть что");
        doc.insert(3, b"ab").unwrap();
        doc.undo().unwrap().expect("отменять есть что");
        doc.undo().unwrap().expect("отменять есть что");
        assert_eq!(doc.read(), b"xy\n");
        doc.insert(2, b"\n").unwrap();
        assert_eq!(doc.read(), b"xy\n\n");
    }

    #[test]
    fn a_snapshot_survives_a_random_history() {
        // Снапшот снимается в произвольный момент под нагрузкой: восстановленный
        // документ должен совпасть и текстом, и историей.
        for seed in 0..4u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
            let mut doc = Document::new("начало".as_bytes());
            for _ in 0..200 {
                let len = doc.len() as usize;
                let pos = rng.below(len + 1) as u32;
                if rng.below(2) == 0 {
                    let _ = doc.delete(
                        pos.min(doc.len().saturating_sub(1)),
                        1 + rng.below(20) as u32,
                    );
                } else {
                    doc.insert(pos, b"qq").unwrap();
                }
                if rng.below(7) == 0 {
                    let _ = doc.undo().unwrap();
                }
            }
            let text = doc.read();
            let captured = history::History::capture(&doc);
            let mut back = captured.restore().expect("снапшот разбирается");
            assert_eq!(back.read(), text, "seed {seed}: текст разошёлся");
            assert_eq!(
                back.path().len(),
                doc.path().len(),
                "seed {seed}: история разошлась"
            );

            // Отмена в восстановленном документе идёт по той же ветке.
            for _ in 0..doc.path().len() {
                back.undo().unwrap().expect("история восстановлена");
            }
            assert_eq!(
                back.read(),
                "начало".as_bytes(),
                "seed {seed}: откат до начала не сошёлся"
            );
        }
    }

    #[test]
    fn errors_speak_through_the_catalog() {
        // Ошибка не несёт готовой строки: текст собирает каталог, иначе её нельзя
        // будет перевести.
        let e = Error::OutOfBounds {
            anchor: 10,
            len: 5,
            doc_len: 12,
        };
        let text = e.message(&crate::messages::En);
        assert!(
            text.contains("15"),
            "в тексте должно быть место значения: {text}"
        );
        assert!(
            text.contains("12"),
            "в тексте должно быть место значения: {text}"
        );
        assert_eq!(e.text(), text);
        assert_eq!(format!("{e}"), text);
    }

    #[test]
    fn an_empty_document_takes_everything() {
        // Пустой документ — не особый случай, но именно на нём чаще всего
        // вылезают ошибки границ.
        let mut doc = Document::new(b"");
        assert!(doc.is_empty());
        assert_eq!(doc.len(), 0);
        assert_eq!(doc.read(), Vec::<u8>::new());
        assert_eq!(doc.read_range(0, 0), Vec::<u8>::new());

        doc.insert(0, "первое".as_bytes()).unwrap();
        assert_eq!(doc.read(), "первое".as_bytes());
        doc.delete(0, doc.len()).unwrap();
        assert!(doc.is_empty(), "весь текст вырезан, документ снова пустой");

        // И отмена возвращает всё как было.
        doc.undo().unwrap().expect("удаление отменяемо");
        assert_eq!(doc.read(), "первое".as_bytes());
    }

    #[test]
    fn deleting_the_whole_document_is_the_same_as_a_new_one() {
        let mut doc = d("одна строка");
        doc.delete(0, doc.len()).unwrap();
        assert!(doc.is_empty());
        assert_eq!(doc.leaves().len(), 0, "листьев не осталось");

        // Дерево пустое, но обязано остаться связным.
        let (nodes, height, skew, mismatch) = doc.tree_debug_stats();
        assert_eq!(nodes, 0);
        assert_eq!(height, 0);
        assert_eq!(skew, 0);
        assert_eq!(mismatch, 0);

        doc.insert(0, "снова".as_bytes()).unwrap();
        assert_eq!(doc.read(), "снова".as_bytes());
    }

    #[test]
    fn a_big_text_is_inserted_in_one_move() {
        // Кусок больше одного листа: вставка идёт одним куском, а читаться и
        // вырезаться он должен так же, как много мелких.
        let chunk: Vec<u8> = (0..50_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let mut doc = d("");
        doc.insert(0, &chunk).unwrap();
        assert_eq!(doc.read(), chunk);
        assert_eq!(doc.read_range(10, 20), chunk[10..20].to_vec());

        doc.delete(0, 25_000).unwrap();
        assert_eq!(doc.read(), chunk[25_000..].to_vec());

        // Вставка в середину большого куска режет его надвое.
        doc.insert(10, "ЖЖ".as_bytes()).unwrap();
        let mut expected = chunk[25_000..].to_vec();
        expected.splice(10..10, "ЖЖ".as_bytes().iter().copied());
        assert_eq!(doc.read(), expected);
        assert!(doc.leaves().len() > 1, "большой кусок не разрезался");
    }

    #[test]
    fn unicode_survives_being_cut_anywhere() {
        // Кириллица — два байта на букву. Резать можно где угодно, включая
        // середину буквы: текст от этого не портится, а длина уменьшается ровно
        // на один байт.
        let text = "Привет, мир! Здравствуйте.";
        let mut doc = d(text);
        assert_eq!(doc.read(), text.as_bytes());
        for at in 0..text.len() as u32 {
            doc.delete(at, 1).unwrap();
            assert_eq!(
                doc.read().len(),
                text.len() - 1,
                "вырезан байт {at}: длина должна уменьшиться ровно на один"
            );
            doc.undo().unwrap().expect("удаление отменяемо");
            assert_eq!(doc.read(), text.as_bytes(), "отмена вернула не то");
        }
    }

    #[test]
    fn a_replace_is_one_step_of_history() {
        // Замена — одно действие, значит и один шаг отмены. Раньше это были две
        // записи, и одна команда отменяла половину работы.
        let mut doc = d("The quick brown fox");
        doc.replace(4, 5, b"slow").unwrap();
        assert_eq!(doc.read(), b"The slow brown fox");
        assert_eq!(doc.path().len(), 1, "в истории одна запись, а не две");

        doc.undo().unwrap().expect("замена отменяема");
        assert_eq!(
            doc.read(),
            b"The quick brown fox",
            "вырезанное вернулось целиком"
        );
        doc.redo().unwrap().expect("замена повторяема");
        assert_eq!(doc.read(), b"The slow brown fox");
    }

    #[test]
    fn a_replace_of_a_shorter_text_is_correct() {
        // Длина меняется в обе стороны: короткий кусок вместо длинного и
        // наоборот.
        let mut doc = d("abcdefghij");
        doc.replace(2, 6, b"XY").unwrap();
        assert_eq!(doc.read(), b"abXYij");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"abcdefghij");

        let mut doc = d("abcdefghij");
        doc.replace(4, 1, b"1234567890").unwrap();
        assert_eq!(doc.read(), b"abcd1234567890fghij");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"abcdefghij");
    }

    #[test]
    fn a_replace_across_leaf_borders_is_correct() {
        // Диапазон замены не влезает ни в один лист: это самый частый случай на
        // большом документе.
        let mut doc = d("");
        for i in 0..40u32 {
            doc.insert(i, format!("{i:03}").as_bytes()).unwrap();
        }
        let before = doc.read();
        let inserted = "замена".as_bytes();
        doc.replace(10, 60, inserted).unwrap();
        assert_eq!(doc.read_range(0, 10), before[..10].to_vec());
        assert_eq!(
            doc.read_range(10, 22),
            inserted,
            "вставлено шесть букв, двенадцать байт"
        );
        assert_eq!(doc.read_range(22, 76), before[70..].to_vec());
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), before, "документ вернулся целиком");
    }

    #[test]
    fn a_replace_survives_the_snapshot() {
        // Замена в истории — самая тяжёлая запись: несёт и вставленное, и
        // вырезанное. После снапшота она обязана разобраться целиком.
        let mut doc = d("исходный текст");
        // Вырезаем только слово, пробел остаётся: «новый текст», а не
        // «новыйтекст».
        let cut = "исходный".len() as u32;
        doc.replace(0, cut, "новый".as_bytes()).unwrap();
        let history = history::History::capture(&doc);
        let mut back = history.restore().expect("снапшот разбирается");
        assert_eq!(back.read(), "новый текст".as_bytes());
        back.undo().unwrap().expect("история живая");
        assert_eq!(back.read(), "исходный текст".as_bytes());
    }

    #[test]
    fn a_replace_of_nothing_is_an_insert() {
        // Замена нулевого диапазона — это вставка, но запись остаётся заменой:
        // так видно, что пользователь нажал команду «заменить».
        let mut doc = d("ab");
        doc.replace(1, 0, b"XY").unwrap();
        assert_eq!(doc.read(), b"aXYb");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"ab");
    }

    #[test]
    fn a_replace_out_of_bounds_is_refused() {
        let mut doc = d("abc");
        assert!(doc.replace(2, 5, b"x").is_err());
        assert!(doc.replace(9, 0, b"x").is_err());
        assert_eq!(doc.read(), b"abc", "документ не тронут");
        assert!(doc.path().is_empty(), "в истории пусто");
    }

    #[test]
    fn every_replace_position_and_length_gives_the_right_text() {
        // Перебор всех позиций и длин на маленьких документах: тут находятся
        // случаи, которые случайная последовательность правок не вылавливает,
        // например пустая замена в начале документа.
        for base_len in 0..6usize {
            let base: String = (0..base_len).map(|i| (b'a' + i as u8) as char).collect();
            for pos in 0..=base_len {
                for len in 0..=(base_len - pos) {
                    let mut doc = Document::new(base.as_bytes());
                    doc.replace(pos as u32, len as u32, b"XYZ").unwrap();
                    let mut want = base.clone();
                    want.replace_range(pos..pos + len, "XYZ");
                    assert_eq!(
                        doc.read(),
                        want.as_bytes(),
                        "основа {base:?}, позиция {pos}, длина {len}"
                    );

                    // И отмена обязана вернуть всё как было.
                    doc.undo().unwrap().expect("замена отменяема");
                    assert_eq!(doc.read(), base.as_bytes(), "отмена вернула не то");
                }
            }
        }
    }

    #[test]
    fn the_replace_survives_a_random_mix_with_undo() {
        // Замена с отменами: структура дерева меняется на каждом шаге, и ошибка
        // проявляется не на ровном куске, а на том, что осталось от прежних
        // правок.
        let mut doc = d("");
        let mut text = String::new();
        let mut log: Vec<String> = Vec::new();
        let mut steps: Vec<(u8, usize, usize, String)> = Vec::new();
        let mut rng = Rng(0x2545F4914F6CDD1D);

        for _ in 0..600 {
            match rng.below(4) {
                0 if !text.is_empty() => {
                    let at = rng.below(text.len());
                    let len = 1 + rng.below((text.len() - at).min(20));
                    let removed = text[at..at + len].to_string();
                    doc.delete(at as u32, len as u32).unwrap();
                    text.replace_range(at..at + len, "");
                    steps.push((0, at, len, removed));
                    log.push(format!("delete {at} {len}"));
                }
                1 => {
                    let at = rng.below(text.len() + 1);
                    let typed = if rng.below(2) == 0 { "\n" } else { "ab" };
                    doc.insert(at as u32, typed.as_bytes()).unwrap();
                    text.insert_str(at, typed);
                    steps.push((1, at, typed.len(), String::new()));
                    log.push(format!("insert {at} {typed:?}"));
                }
                2 => {
                    let at = rng.below(text.len() + 1);
                    let len = rng.below((text.len() - at).min(10));
                    let removed = text[at..at + len].to_string();
                    doc.replace(at as u32, len as u32, b"xy\n").unwrap();
                    text.replace_range(at..at + len, "xy\n");
                    steps.push((2, at, 3, removed));
                    log.push(format!("replace {at} {len}"));
                }
                _ => {
                    let Some((kind, at, len, removed)) = steps.pop() else {
                        continue;
                    };
                    doc.undo().unwrap().expect("отменять есть что");
                    match kind {
                        // Отмена вставки: убрать вставленное.
                        1 => text.replace_range(at..at + len, ""),
                        // Отмена удаления: вернуть вырезанное.
                        0 => text.insert_str(at, &removed),
                        // Отмена замены: убрать вставленное и вернуть
                        // вырезанное.
                        _ => {
                            text.replace_range(at..at + len, "");
                            text.insert_str(at, &removed);
                        }
                    }
                    log.push(format!("undo {at} {len}"));
                }
            }

            assert_eq!(
                doc.read(),
                text.as_bytes(),
                "разошлось на {}: {:?}\nвся последовательность: {:?}",
                log.last().unwrap(),
                text,
                log
            );
        }
    }
}

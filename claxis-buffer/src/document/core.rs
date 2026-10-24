use std::path::PathBuf;

use super::error::{DEFAULT_DEPTH, Error, MIN_DEPTH, Result, SnapshotSink};
use super::record::{Record, Version};
use crate::node::NIL;
use crate::tree::Tree;

/// Сколько вершин истории заказывается вперёд при сборке документа.
///
/// Столько же, сколько правок живёт до снимка при глубине по умолчанию: ещё
/// одна такая же порция — и память пришлось бы просить заново. Больше брать
/// нельзя: глубина настраивается и потолка не имеет.
const VERSION_RESERVE: usize = 8 * 1024;

/// Минимальный шаг роста текстового буфера.
pub(super) const TEXT_GROWTH_MIN: usize = 4 * 1024;

/// Запас под правки для буфера размера `size`.
///
/// Кривая убывает по логарифму размера: мелкий файл растёт вдвое, крупный —
/// на проценты. Так мелкий буфер редко перевыделяется, а большой не держит
/// лишние мегабайты только из-за коэффициента.
pub(super) fn text_growth_margin(size: usize) -> usize {
    if size == 0 {
        return TEXT_GROWTH_MIN;
    }
    let log = (size as f64).log2();
    let factor = if log <= 14.0 {
        2.0
    } else if log <= 18.0 {
        2.0 - 0.5 * (log - 14.0) / 4.0
    } else if log <= 19.0 {
        1.5 - 0.25 * (log - 18.0)
    } else if log <= 20.0 {
        1.25 - 0.05 * (log - 19.0)
    } else if log <= 24.0 {
        1.2 - 0.1 * (log - 20.0) / 4.0
    } else if log <= 30.0 {
        1.1 - 0.09 * (log - 24.0) / 6.0
    } else {
        1.01
    };
    let margin = (size as f64 * (factor - 1.0)).round() as usize;
    margin.max(TEXT_GROWTH_MIN)
}

/// Длина текста, влезающая в 32-битное смещение буфера.
///
/// Проверка стоит до `as u32`: иначе длина обрезалась бы молча, и хвост
/// документа исчез бы вместе с историей о нём. Вынесена отдельно, чтобы
/// переполнение проверялось без настоящей четырёхгигабайтной аллокации.
fn checked_len(bytes: usize) -> Result<u32> {
    if bytes > u32::MAX as usize {
        return Err(Error::BufferTooLarge { len: bytes as u64 });
    }
    Ok(bytes as u32)
}

/// Документ: текст, история и снапшоты.
pub struct Document {
    /// Байты под деревом: исходный файл в начале, дальше вставки и вырезанное.
    ///
    /// Именно байты, а не текст документа: сам текст собирается обходом дерева,
    /// и путать эти две вещи нельзя. Публичное имя того же — `buffer`.
    pub(super) bytes: Vec<u8>,
    /// Граница исходного файла: вставки идут после неё.
    pub(super) file_len: u32,
    pub(super) tree: Tree,
    pub(super) versions: Vec<Version>,
    pub(super) free: Vec<u32>,
    pub(super) current: u32,
    /// Глубина истории.
    pub(super) depth: u32,
    pub(super) sink: Option<Box<dyn SnapshotSink>>,
    pub(super) file: Option<PathBuf>,
    pub(super) snapshots: u64,
    /// Сколько правок было в памяти на момент последнего снимка.
    pub(super) snapshot_mark: u32,
    /// Сколько раз документ схлопывался в базу после сохранения.
    pub(super) compacts: u64,
}

impl Document {
    /// Документ с текстом и глубиной истории по умолчанию.
    ///
    /// Глубина по умолчанию задана самим крейтом, проверять её нечего. А вот
    /// длину текста проверить надо: смещения в буфере 32-битные, и текст
    /// длиннее [`u32::MAX`] байт не влезает. Тихая обрезка до `u32` потеряла
    /// бы хвост документа, поэтому переполнение — ошибка.
    pub fn new(bytes: impl AsRef<[u8]>) -> Result<Self> {
        Self::with_depth(bytes, DEFAULT_DEPTH)
    }

    /// Документ с заданной глубиной истории.
    pub fn with_depth(bytes: impl AsRef<[u8]>, depth: u32) -> Result<Self> {
        if depth < MIN_DEPTH {
            return Err(Error::InvalidHistoryDepth { depth });
        }
        let source = bytes.as_ref();
        let len = checked_len(source.len())?;
        let capacity = if source.is_empty() {
            0
        } else {
            source
                .len()
                .saturating_add(text_growth_margin(source.len()))
        };
        let mut copy = Vec::with_capacity(capacity);
        copy.extend_from_slice(source);
        Ok(Self::assemble(copy, len, depth))
    }

    /// Документ из готового буфера: текст уже состоит из файла и хвоста.
    ///
    /// Основание — первые `file_len` байт, остальное хвост: так записи
    /// получают верные смещения, а документ остаётся ровно файлом.
    pub fn from_buffer(bytes: Vec<u8>, file_len: usize) -> Self {
        Self::from_buffer_with_depth(bytes, file_len, DEFAULT_DEPTH)
    }

    /// Документ из готового буфера с заданной глубиной истории.
    ///
    /// Нужен восстановлению: история не хранит глубину, и без неё документ
    /// после разбора снимка терял бы настройку и снимал снимки не тогда.
    pub(crate) fn from_buffer_with_depth(bytes: Vec<u8>, file_len: usize, depth: u32) -> Self {
        let file_len = file_len.min(bytes.len());
        let file_len = file_len as u32;
        Self::assemble(bytes, file_len, depth)
    }

    fn assemble(bytes: Vec<u8>, file_len: u32, depth: u32) -> Self {
        // Вершин до снимка будет не больше, чем глубина; под обычную глубину
        // память берётся сразу, чтобы правки не платили за реаллокацию. Но
        // потолка у глубины нет — [`Document::with_depth`] принимает любое
        // число, — поэтому запас ограничен: иначе `depth` в миллиарды просил
        // бы гигабайты под пустую историю.
        let reserve = depth.min(VERSION_RESERVE as u32) as usize;
        let mut versions = Vec::with_capacity(reserve);
        versions.push(Version {
            record: Record::Delete {
                anchor: 0,
                head: 0,
                text_offset: 0,
            },
            parent: NIL,
            child: NIL,
            live: 0,
        });
        let mut document = Self {
            tree: Tree::with_leaf(0, file_len),
            bytes,
            file_len,
            versions,
            // Свободных вершин бывает не больше, чем занятых: запас тот же.
            free: Vec::with_capacity(reserve),
            current: 0,
            depth,
            sink: None,
            file: None,
            snapshots: 0,
            snapshot_mark: 0,
            compacts: 0,
        };
        document.reserve_text_headroom();
        document
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

    // ── чтение ───────────────────────────────────────────────────────────

    /// Текст диапазона документа.
    pub(super) fn slice(&self, start: u32, end: u32) -> Vec<u8> {
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
            // Кусок в буфере не длиннее самого диапазона: столько и просим,
            // чтобы не реаллоцировать на каждом листе.
            out: Vec::with_capacity(end.saturating_sub(start) as usize),
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

    /// Диапазон документа прямо в приёмник, без промежуточного `Vec`.
    ///
    /// За документ не выходит: лишнее отбрасывает дерево, как и у
    /// [`Document::read_range`].
    pub fn read_range_into<S: crate::Sink>(&self, start: u32, end: u32, sink: &mut S) {
        self.tree.read_range_into(start, end, sink);
    }

    /// Весь документ прямо в приёмник, без промежуточного `Vec`.
    pub fn read_into<S: crate::Sink>(&self, sink: &mut S) {
        self.tree.read_range_into(0, self.len(), sink);
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
        // Ветка не длиннее числа занятых вершин: столько и просим.
        let mut out = Vec::with_capacity(self.history_used() as usize);
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
pub(super) mod support {
    use super::super::{Document, SnapshotSink};
    pub use crate::history;

    /// Кольцо снимков в памяти: пишется всё, отдаётся по убыванию.
    ///
    /// Так же устроено кольцо на диске, только без второй копии под рукой.
    #[derive(Debug, Default)]
    pub struct Ring {
        pub written: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
    }

    /// Приёмник, умеющий делиться кольцом с тестом: приёмник уезжает внутрь
    /// документа, а кольцо нужно и там, и снаружи.
    #[derive(Debug, Clone)]
    pub struct Share(pub std::rc::Rc<std::cell::RefCell<Ring>>);

    impl SnapshotSink for Share {
        fn accept(&mut self, path: &std::path::Path, text: &[u8], history: &[u8]) -> bool {
            self.0
                .borrow_mut()
                .written
                .push((path.to_path_buf(), text.to_vec(), history.to_vec()));
            true
        }
    }

    /// Приёмник для тестов, которым нужен только привязанный файл: снимок
    /// принимается и запоминается, но кольца нет — старее снимков не бывает.
    #[derive(Debug, Default)]
    pub struct Attached {
        pub accepted: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
    }

    impl SnapshotSink for Attached {
        fn accept(&mut self, path: &std::path::Path, text: &[u8], history: &[u8]) -> bool {
            self.accepted
                .push((path.to_path_buf(), text.to_vec(), history.to_vec()));
            true
        }
    }

    /// Приёмник, который не смог записать снимок: `accept` отвечает `false`.
    ///
    /// Так выглядит поломка диска: снимок не сохранён, и история обязана
    /// остаться в памяти целиком.
    #[derive(Debug, Default)]
    pub struct Refusing {
        /// Что приёмнику предлагали записать. Вход не пропадает молча даже у
        /// сломанного приёмника.
        pub offered: Vec<(std::path::PathBuf, Vec<u8>, Vec<u8>)>,
    }

    impl SnapshotSink for Refusing {
        fn accept(&mut self, path: &std::path::Path, text: &[u8], history: &[u8]) -> bool {
            self.offered
                .push((path.to_path_buf(), text.to_vec(), history.to_vec()));
            false
        }
    }

    pub fn d(text: &str) -> Document {
        Document::new(text.as_bytes()).expect("valid document")
    }

    /// Генератор чисел: детерминированный, чтобы падение воспроизводилось.
    pub struct Rng(pub u64);

    impl Rng {
        pub fn below(&mut self, n: usize) -> usize {
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
    pub enum Op {
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
}

#[cfg(test)]
mod tests {
    //! Проверки документа: правки, отмены, снапшоты и баланс дерева.
    //!
    //! Главная проверка — сверка с эталоном на длинной случайной
    //! последовательности правок с перемешиванием отмен и повторов. Эталон —
    //! обычная строка, и любое расхождение означает битый документ.

    use super::support::*;
    use super::*;

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
    fn reading_into_a_sink_matches_the_vec() {
        // Приёмник сам режет диапазоны по буферу: то же, что делает `slice`,
        // но без промежуточного `Vec` на стороне документа.
        struct Copy<'a> {
            bytes: &'a [u8],
            out: Vec<u8>,
        }
        impl crate::Sink for Copy<'_> {
            fn range(&mut self, start: u32, len: u32) {
                let from = (start as usize).min(self.bytes.len());
                let to = (start as usize)
                    .saturating_add(len as usize)
                    .min(self.bytes.len());
                self.out.extend_from_slice(&self.bytes[from..to]);
            }
        }

        let mut doc = d("hello, world");
        doc.insert(doc.len(), b"!").unwrap();
        doc.delete(0, 2).unwrap();

        let mut whole = Copy {
            bytes: doc.buffer(),
            out: Vec::new(),
        };
        doc.read_into(&mut whole);
        assert_eq!(whole.out, doc.read());

        let mut part = Copy {
            bytes: doc.buffer(),
            out: Vec::new(),
        };
        doc.read_range_into(3, 9, &mut part);
        assert_eq!(part.out, doc.read_range(3, 9));
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
    fn an_empty_document_takes_everything() {
        // Пустой документ — не особый случай, но именно на нём чаще всего
        // вылезают ошибки границ.
        let mut doc = Document::new(b"").unwrap();
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
        let stats = doc.tree_debug_stats().expect("дерево не зациклено");
        assert_eq!(stats.nodes, 0);
        assert_eq!(stats.height, 0);
        assert_eq!(stats.worst_imbalance, 0);
        assert_eq!(stats.height_mismatch, 0);

        doc.insert(0, "снова".as_bytes()).unwrap();
        assert_eq!(doc.read(), "снова".as_bytes());
    }

    #[test]
    fn text_longer_than_a_u32_is_rejected() {
        // Смещения буфера 32-битные: текст длиннее `u32::MAX` байт не влезает,
        // и обрезать его молча нельзя. Проверяется без настоящей аллокации —
        // через ту же функцию, что стоит в `with_depth`.
        assert_eq!(checked_len(u32::MAX as usize), Ok(u32::MAX));
        assert_eq!(
            checked_len(u32::MAX as usize + 1),
            Err(Error::BufferTooLarge {
                len: u32::MAX as u64 + 1
            })
        );
    }

    /// Проверить устройство документа: текст, дерево и буфер разом.
    ///
    /// Это не сверка с эталоном, а проверка внутренней согласованности. Она
    /// ловит то, чего не видит один лишь текст: дерево может разъехаться по
    /// высоте или уронить лист за буфер и при этом ещё несколько шагов
    /// читаться верно, прежде чем развалиться.
    fn assert_sound(doc: &Document, text: &[u8], what: &str) {
        assert_eq!(doc.read(), text, "{what}: текст разошёлся");
        assert_eq!(doc.len() as usize, text.len(), "{what}: длина разошлась");

        let stats = doc.tree_debug_stats().expect("дерево не зациклено");
        assert!(doc.tree_is_sound(), "{what}: дерево не связно");
        assert_eq!(stats.height_mismatch, 0, "{what}: высота в узле разошлась");
        assert_eq!(
            stats.height,
            doc.height(),
            "{what}: высота корня не сошлась"
        );
        assert!(
            stats.worst_imbalance <= 1,
            "{what}: перекос {}",
            stats.worst_imbalance
        );

        let mut total = 0u64;
        for leaf in doc.leaves() {
            assert!(leaf.len > 0, "{what}: пустой лист {leaf:?}");
            let end = leaf.start as u64 + leaf.len as u64;
            assert!(
                end <= doc.buffer_len() as u64,
                "{what}: лист {leaf:?} вышел за буфер длиной {}",
                doc.buffer_len()
            );
            total += leaf.len as u64;
        }
        assert_eq!(
            total,
            text.len() as u64,
            "{what}: листья не покрывают текст"
        );
    }

    #[test]
    fn the_document_stays_sound_under_random_edits() {
        // Сверка с эталоном ловит неверный текст, но не поломку устройства:
        // дерево может разъехаться по высоте и всё ещё читаться верно. Здесь
        // после каждой правки, отмены и повтора проверяется и текст, и дерево,
        // и буфер.
        for seed in 0..12u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
            let mut doc = Document::new(b"base").unwrap();
            let mut text = b"base".to_vec();
            let mut history: Vec<Op> = Vec::new();
            let mut cursor = 0usize;

            for step in 0..800 {
                let what = format!("seed {seed} step {step}");
                match rng.below(11) {
                    0..=3 if !text.is_empty() => {
                        let pos = rng.below(text.len());
                        let len = 1 + rng.below((text.len() - pos).min(24));
                        let removed = text[pos..pos + len].to_vec();
                        doc.delete(pos as u32, len as u32).unwrap();
                        text.drain(pos..pos + len);
                        history.truncate(cursor);
                        history.push(Op::Delete { pos, removed });
                        cursor = history.len();
                    }
                    4..=6 => {
                        let pos = rng.below(text.len() + 1);
                        let typed: Vec<u8> = (0..1 + rng.below(16))
                            .map(|_| b'a' + rng.below(26) as u8)
                            .collect();
                        doc.insert(pos as u32, &typed).unwrap();
                        text.splice(pos..pos, typed.iter().copied());
                        history.truncate(cursor);
                        history.push(Op::Insert { pos, text: typed });
                        cursor = history.len();
                    }
                    7..=8 => {
                        let pos = rng.below(text.len() + 1);
                        let len = rng.below((text.len() - pos).min(24));
                        let removed = text[pos..pos + len].to_vec();
                        let typed: Vec<u8> = (0..1 + rng.below(12))
                            .map(|_| b'a' + rng.below(26) as u8)
                            .collect();
                        doc.replace(pos as u32, len as u32, &typed).unwrap();
                        text.splice(pos..pos + len, typed.iter().copied());
                        history.truncate(cursor);
                        history.push(Op::Replace {
                            pos,
                            removed,
                            text: typed,
                        });
                        cursor = history.len();
                    }
                    9 if cursor > 0 => {
                        cursor -= 1;
                        doc.undo().unwrap().expect("отменять есть что");
                        match &history[cursor] {
                            Op::Insert { pos, text: t } => {
                                text.drain(*pos..*pos + t.len());
                            }
                            Op::Delete { pos, removed } => {
                                text.splice(*pos..*pos, removed.iter().copied());
                            }
                            Op::Replace {
                                pos,
                                removed,
                                text: t,
                            } => {
                                text.drain(*pos..*pos + t.len());
                                text.splice(*pos..*pos, removed.iter().copied());
                            }
                        }
                    }
                    _ if cursor < history.len() => {
                        doc.redo().unwrap().expect("повтор доступен");
                        match &history[cursor] {
                            Op::Insert { pos, text: t } => {
                                text.splice(*pos..*pos, t.iter().copied());
                            }
                            Op::Delete { pos, removed } => {
                                text.drain(*pos..*pos + removed.len());
                            }
                            Op::Replace {
                                pos,
                                removed,
                                text: t,
                            } => {
                                text.splice(*pos..*pos + removed.len(), t.iter().copied());
                            }
                        }
                        cursor += 1;
                    }
                    _ => continue,
                }
                assert_sound(&doc, &text, &what);
                assert_eq!(doc.path().len(), cursor, "{what}: история разошлась");
            }
        }
    }

    #[test]
    fn random_edits_survive_a_history_round_trip() {
        // Снапшот уходит байтами и приходит байтами же. После случайных правок
        // восстановленный документ обязан читаться так же и отменяться
        // синхронно с исходным — до самого основания.
        for seed in 0..8u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B54A32D192ED03) | 1);
            let mut doc = Document::new(b"base").unwrap();
            let mut text = b"base".to_vec();
            for _ in 0..300 {
                let roll = rng.below(10);
                if roll < 3 && !text.is_empty() {
                    let pos = rng.below(text.len());
                    let len = 1 + rng.below((text.len() - pos).min(20));
                    doc.delete(pos as u32, len as u32).unwrap();
                    text.drain(pos..pos + len);
                } else if roll < 7 {
                    let pos = rng.below(text.len() + 1);
                    let typed: Vec<u8> = (0..1 + rng.below(12))
                        .map(|_| b'a' + rng.below(26) as u8)
                        .collect();
                    doc.insert(pos as u32, &typed).unwrap();
                    text.splice(pos..pos, typed.iter().copied());
                } else if !text.is_empty() {
                    let pos = rng.below(text.len());
                    let len = 1 + rng.below((text.len() - pos).min(20));
                    let typed: Vec<u8> = (0..1 + rng.below(12))
                        .map(|_| b'a' + rng.below(26) as u8)
                        .collect();
                    doc.replace(pos as u32, len as u32, &typed).unwrap();
                    text.splice(pos..pos + len, typed.iter().copied());
                }
            }

            let expected = doc.read();
            assert_eq!(expected, text, "seed {seed}: эталон разошёлся с документом");

            let stored = history::History::capture(&doc);
            let mut back = history::History::decode(stored.as_bytes())
                .expect("блоб разбирается")
                .restore(b"base", DEFAULT_DEPTH)
                .expect("документ собирается");
            assert_eq!(
                back.read(),
                expected,
                "seed {seed}: текст не пережил снапшот"
            );
            assert_sound(&back, &expected, &format!("seed {seed} из снапшота"));

            // Отменяем оба в ногу: история одна, значит и текст на каждом шаге
            // один.
            loop {
                let left = doc.undo().unwrap();
                let right = back.undo().unwrap();
                assert_eq!(
                    left.is_some(),
                    right.is_some(),
                    "seed {seed}: истории разной длины"
                );
                if left.is_none() {
                    break;
                }
                assert_eq!(doc.read(), back.read(), "seed {seed}: отмена разошлась");
            }
            assert_eq!(
                doc.read(),
                b"base",
                "seed {seed}: отмена не дошла до основания"
            );
        }
    }
}

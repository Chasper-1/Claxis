//! Редакторская сессия: хранилище снапшотов и открытые документы.
//!
//! Сессия держит хранилище и настройки снапшотов и открывает документы:
//! читает файл, восстанавливает историю из базы и решает, можно ли писать.

use std::path::PathBuf;
use std::time::Duration;

use crate::api::buffer::defaults::buffer::HISTORY_DEPTH;
use crate::api::buffer::{Error as RopeError, History, MIN_DEPTH};
use crate::api::disk::{KEEP, PERSIST, SnapshotStore, StorePaths, file_len, hash, read_bytes};
use crate::api::save;
use crate::router::document::Document;
use crate::router::error::SessionError;
use crate::router::lines::Lines;
use crate::router::sink::StoreSink;

/// Снапшот по умолчанию: раз в пять минут.
pub const DEFAULT_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(300);

/// Редакторская сессия: хранилище снапшотов и открытые документы.
#[derive(Debug)]
pub struct Session {
    store: SnapshotStore,
    paths: StorePaths,
    /// Сколько записей правки живёт в памяти до снапшота. Из конфига.
    history_depth: u32,
    /// Писать ли снапшоты на диск. Из конфига.
    persist: bool,
    /// Через сколько снимать следующий снапшот. Из конфига.
    interval: Duration,
    /// Сколько секунд правкам положено жить. Из конфига.
    ///
    /// Ноль означает «срока нет»: чистка идёт только по сохранению и выходу.
    retention: u64,
    /// Глубина, с которой сессия открыта и под которую созданы буферы.
    ///
    /// Отличается от `history_depth`, если конфиг поменяли на лету: буферы
    /// уже созданы под прежнюю, и пересоздать их без потери истории нельзя.
    opened_depth: u32,
}

/// Что применилось сразу, а что ждёт перезапуска.
///
/// Не всё настраивается на лету: глубина истории задана при создании буфера,
/// и сменить её можно только пересоздав буфер вместе с историей. Лучше сказать
/// пользователю, что нужен перезапуск, чем применить наполовину и сделать
/// вид, что всё в порядке.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Ключи, подхваченные сразу.
    pub applied: Vec<&'static str>,
    /// Ключи, которые заработают после перезапуска.
    pub needs_restart: Vec<&'static str>,
}

impl Session {
    /// Открыть сессию по настройкам по умолчанию.
    ///
    /// Значения берутся из настроек крейтов: `keep` и `persist` — из стора,
    /// глубина истории — из буфера. Когда появится конфиг, он подставит сюда
    /// свои значения, и менять тут ничего не придётся.
    pub fn open_default() -> Result<Self, SessionError> {
        Self::open(KEEP, PERSIST, HISTORY_DEPTH.value)
    }

    /// Открыть сессию с заданными `keep` и глубиной истории.
    pub fn open(keep: u8, persist: bool, history_depth: u32) -> Result<Self, SessionError> {
        let paths = StorePaths::from_env();
        Self::build(paths, keep, persist, history_depth)
    }

    fn build(
        paths: StorePaths,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        let store = SnapshotStore::open(&paths, keep).map_err(|e| SessionError::Store {
            path: paths.database().display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            store,
            paths,
            history_depth,
            persist,
            interval: DEFAULT_SNAPSHOT_INTERVAL,
            // Неделя: та же неделя, что у умолчания крейта сохранения, и
            // совпадение сторожит тест в этом файле. В константе — потому,
            // что разбор строки в константу не укладывается.
            retention: 7 * 24 * 60 * 60,
            opened_depth: history_depth,
        })
    }

    /// Открыть сессию с хранилищем в заданном каталоге — нужно в тестах.
    pub fn open_at(dir: impl Into<PathBuf>, keep: u8) -> Result<Self, SessionError> {
        Self::open_at_with(dir, keep, PERSIST, HISTORY_DEPTH.value)
    }

    /// То же, но с явными `persist` и глубиной истории.
    pub fn open_at_with(
        dir: impl Into<PathBuf>,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::build(StorePaths::new(dir), keep, persist, history_depth)
    }

    /// То же, но путь принимается как ссылка.
    pub fn open_at_ref(
        dir: &std::path::Path,
        keep: u8,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::open_at_with(dir.to_path_buf(), keep, persist, history_depth)
    }

    /// Сколько снапшотов одного файла хранится. Ноль — только память.
    pub fn keep(&self) -> u8 {
        self.store.keep()
    }

    /// Пишутся ли снапшоты на диск.
    pub fn persist(&self) -> bool {
        self.persist
    }

    /// Через сколько после последнего снапшота снимать следующий.
    pub fn snapshot_interval(&self) -> Duration {
        self.interval
    }

    /// Задать интервал между снапшотами. Применяется к документам, которые
    /// откроются дальше; уже открытые держат то, что получили при открытии.
    pub fn set_snapshot_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// Задать срок годности правок в секундах. Ноль — срока нет.
    ///
    /// Применяется к документам, которые откроются дальше: чистка происходит
    /// при открытии файла, и уже открытые свой срок отработали.
    pub fn set_snapshots_retention(&mut self, retention: u64) {
        self.retention = retention;
    }

    /// Сколько секунд правкам положено жить.
    pub fn snapshots_retention(&self) -> u64 {
        self.retention
    }

    /// Записать документ в его файл через ворота проверки.
    ///
    /// Решение остаётся у политики сохранения: обе ступени против файла на
    /// диске, и при отказе на диск ничего не уезжает. Даже при отказе ошибка
    /// не рождается: отказ — ответ, а не поломка. Возвращает итог с байтом
    /// записанного: рассказать, сколько на диске стало.
    pub fn save_document(&self, document: &Document) -> Result<save::Write, SessionError> {
        let path = document.path();
        save::write(&self.store, path, &document.text()).map_err(|e| SessionError::Save {
            path: path.display().to_string(),
            reason: e.text(),
        })
    }

    /// Отметить документ записанным.
    pub fn mark_saved(&self, document: &mut Document) {
        document.mark_saved();
    }

    /// Забыть в базе и якорь, и все правки файла.
    ///
    /// После успешного сохранения и выхода: работа уже в файле, и истории
    /// вокруг файла дальше держать незачем.
    pub fn forget_snapshots(&self, path: impl AsRef<std::path::Path>) -> Result<(), SessionError> {
        let path = path.as_ref();
        self.store.forget(path).map_err(|e| SessionError::Save {
            path: path.display().to_string(),
            reason: e.text(),
        })?;
        self.store
            .forget_anchor(path)
            .map_err(|e| SessionError::Save {
                path: path.display().to_string(),
                reason: e.text(),
            })
    }

    /// Глубина истории из последнего прочитанного конфига.
    pub fn history_depth(&self) -> u32 {
        self.history_depth
    }

    /// Глубина, под которую реально созданы буферы.
    ///
    /// Расходится с `history_depth`, если конфиг поменяли на лету: применять
    /// такой ключ можно только перезапуском.
    pub fn opened_depth(&self) -> u32 {
        self.opened_depth
    }

    /// Глубина для новых документов.
    ///
    /// Конфиг может назвать ноль, а буфер с таким значением не создаётся:
    /// редактор обязан сказать об этом ошибкой, а не приводить значение
    /// молча — человек у себя в конфиге такого не писал.
    fn history_depth_for_new_documents(&self) -> Result<u32, SessionError> {
        if self.history_depth < MIN_DEPTH {
            return Err(SessionError::Buffer(RopeError::InvalidHistoryDepth {
                depth: self.history_depth,
            }));
        }
        Ok(self.history_depth)
    }

    /// Где лежит хранилище.
    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// Хранилище снапшотов.
    pub fn store(&self) -> &SnapshotStore {
        &self.store
    }

    /// Применить настройки без перезапуска редактора.
    ///
    /// Конфиг сохранили — редактор перечитал его и применяет. Не всё можно
    /// применить на лету, и сказать об этом лучше, чем применить наполовину.
    ///
    /// `snapshots_keep` применяется сразу: это число вокруг которого
    /// выбираются снапшоты. `snapshots_persist` — сразу для документов,
    /// которые откроются дальше; уже открытые держат то, что получили при
    /// открытии. `history_depth` задаёт, сколько правок живёт в памяти до
    /// снапшота, и у открытого документа не меняется.
    pub fn apply(&mut self, settings: &crate::router::Settings) -> Applied {
        let mut applied = Applied::default();

        if self.store.keep() != settings.snapshots_keep {
            self.store.set_keep(settings.snapshots_keep);
            applied.applied.push("snapshots_keep");
        }
        if self.persist != settings.snapshots_persist {
            self.persist = settings.snapshots_persist;
            applied.applied.push("snapshots_persist");
        }
        if self.history_depth != settings.history_depth {
            self.history_depth = settings.history_depth;
            applied.needs_restart.push("history_depth");
        }
        if self.retention != settings.snapshots_retention {
            self.retention = settings.snapshots_retention;
            applied.applied.push("snapshots_days");
        }

        applied
    }

    /// Открыть документ: прочитать файл и привязать снапшоты к хранилищу.
    ///
    /// Файла может не быть — тогда это новый пустой документ, ошибка не
    /// возникает.
    pub fn open_file(&self, path: impl AsRef<std::path::Path>) -> Result<Document, SessionError> {
        let path = path.as_ref().to_path_buf();

        // Проверяем размер до чтения: буфер работает с u32, файл больше
        // u32::MAX байт не помещается.
        let read_error = |e: crate::api::disk::Error| SessionError::ReadFile {
            path: path.display().to_string(),
            reason: e.text(),
        };
        if let Some(len) = file_len(&path).map_err(read_error)?
            && len > u32::MAX as u64
        {
            return Err(SessionError::FileTooLarge { bytes: len });
        }

        let text = read_bytes(&path).map_err(read_error)?.unwrap_or_default();
        let mut buffer = crate::api::buffer::Document::with_depth(
            text.clone(),
            self.history_depth_for_new_documents()?,
        )
        .map_err(SessionError::Buffer)?;
        let mut changed_outside = false;
        let mut anchor_lost = false;
        // Снапшот хранит только правки поверх файла. Исходный текст уже
        // прочитан с диска, поэтому история накладывается на него, а не
        // заменяет его.
        if self.persist
            && let Ok(store) = self.store.reopen()
        {
            // Срок годности отрабатывает при открытии: просроченные правки
            // из базы уходят.
            if self.retention > 0
                && let Err(e) = store.expire(&path, self.retention)
            {
                return Err(SessionError::Store {
                    path: self.paths.database().display().to_string(),
                    reason: e.text(),
                });
            }
            let snapshot = store.latest(&path).ok().flatten();
            let anchor = store.anchor(&path).map_err(|e| SessionError::Store {
                path: self.paths.database().display().to_string(),
                reason: e.to_string(),
            })?;
            // Якоря нет, а правки лежат: якорь пропал, файл мог измениться.
            // Новым на его место живое содержимое не выдумывается: сохранение
            // у такого файла будет категорически отказываться писать, пока
            // правки не выйдут по сроку.
            let file_agrees = match anchor {
                None if snapshot.is_some() => {
                    anchor_lost = true;
                    false
                }
                // Файла нет: буфер пуст, с основой не сверяешь.
                None => true,
                Some(anchor) => {
                    // Ступень размера: почти всё решается здесь и бесплатно.
                    match file_len(&path).map_err(read_error)? {
                        // Файла нет. Якорь нулевой длины — файла и не было,
                        // меняться нечему; якорь с длиной — файл исчез.
                        None => anchor.len == 0,
                        // Разошёлся размер — правка извне видна сразу.
                        Some(len) if len != anchor.len => false,
                        // Размер сошёлся — платит хеш: правка той же длины
                        // теперь действительно отчёта требует.
                        Some(len) => match hash::of_file(&path).map_err(read_error)? {
                            Some(on_disk) => save::agrees(&anchor, len, on_disk),
                            None => false,
                        },
                    }
                }
            };
            // Чужой или битый блоб историю не восстановит — и не должен:
            // файл-то на месте, терять его из-за испорченного хвоста незачем.
            if file_agrees
                && let Some(ref snapshot) = snapshot
                && let Ok(history) = History::decode(&snapshot.history)
            {
                match history.matches_base(&text) {
                    // Файл тот же, на котором снят снапшот: применяем.
                    Ok(true) => {
                        // Глубина берётся у уже открытого буфера: в снимке
                        // её нет, а восстановленный документ должен
                        // снимать снимки по настройке, а не по умолчанию.
                        if let Ok(restored) = history.restore(&text, buffer.history_depth()) {
                            buffer = restored;
                        }
                    }
                    // Основа истории не тот текст, что на диске, хотя якорь
                    // сходится: история старше файла. Накладывать нельзя,
                    // но и тревожить не о чем — якорь за файл ручается.
                    Ok(false) => {}
                    Err(_) => {}
                }
            }
            // Файл не тот, на котором снят снапшот: применять нельзя, правка
            // извне стала бы невидимой и уехала бы под следующим
            // сохранением. Открываем файл как есть и говорим об этом.
            if !file_agrees && snapshot.is_some() {
                changed_outside = true;
            }
            // Якорь ставится заново, если буфер видит сейчас другое
            // содержимое: правка извне стала основой, или якоря не было.
            // При пропавшем якоре новый на месте правок не выдумывается:
            // пока правки живы, записываться файлу нельзя честно.
            if !anchor_lost && (anchor.is_none() || !file_agrees) {
                save::reanchor(&store, &path, &text).map_err(|e| SessionError::Store {
                    path: self.paths.database().display().to_string(),
                    reason: e.message(&crate::api::save::messages::En),
                })?;
            }
        }
        // Приёмник вешается только если снапшоты положено хранить. Ноль в кольце —
        // это «только память»: приёмника не будет вовсе, иначе он писал бы
        // в базу строку, которую кольцо тут же удаляет, а пользователь с
        // `persist = true` думал бы, что работа переживает перезапуск.
        let sink_failed = if self.persist && self.store.keep() > 0 {
            // Своё подключение к той же базе: буфер живёт дольше сессии.
            let store = self.store.reopen().map_err(|e| SessionError::Store {
                path: self.paths.database().display().to_string(),
                reason: e.to_string(),
            })?;
            let failed = std::rc::Rc::new(std::cell::RefCell::new(None));
            buffer.set_sink(
                path.clone(),
                Box::new(StoreSink::new(store, std::rc::Rc::clone(&failed))),
            );
            Some(failed)
        } else {
            None
        };
        // Без приёмника ошибки писать некуда: ячейка всё равно нужна, чтобы
        // документ ждал одинаково в обоих случаях.
        let sink_failed = sink_failed.unwrap_or_default();
        Ok(Document::from_parts(crate::router::document::Parts {
            path,
            // Карта строится по тексту, а не едет дельтами: документ только что
            // открыт, истории правок ещё не было.
            lines: Lines::build(&buffer.read()),
            buffer,
            changed_outside,
            anchor_lost,
            sink_failed,
            interval: self.interval,
            clock: Box::new(crate::router::clock::SystemClock),
        }))
    }
}

/// Приспособления для тестов: общие на несколько файлов.
#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use crate::router::clock::FixedClock;
    use crate::router::session::Session;

    /// Каталог под тест: базы не делятся, и каталог убирается вместе с собой.
    pub struct Temp {
        /// Хранилище каталога: убирает его при выходе из теста.
        inner: crate::api::disk::temp::Temp,
    }

    impl Temp {
        /// Новый каталог под тест.
        pub fn new(name: &str) -> Self {
            Self {
                inner: crate::api::disk::temp::Temp::new(name),
            }
        }

        /// Каталог, в котором живёт база и файлы теста.
        pub fn dir(&self) -> &Path {
            self.inner.path()
        }

        /// Файл документа рядом с базой.
        pub fn file(&self, name: &str) -> PathBuf {
            self.dir().join(name)
        }

        /// Сессия с обычной глубиной истории.
        pub fn session(&self, keep: u8) -> Session {
            Session::open_at(self.dir().to_path_buf(), keep).unwrap()
        }

        /// Сессия с короткой историей: снапшот срабатывает быстро.
        pub fn short_session(&self, keep: u8, depth: u32) -> Session {
            Session::open_at_with(self.dir().to_path_buf(), keep, true, depth).unwrap()
        }

        /// Сессия, которая не пишет снапшоты на диск.
        pub fn no_persist_session(&self, keep: u8, depth: u32) -> Session {
            Session::open_at_with(self.dir().to_path_buf(), keep, false, depth).unwrap()
        }
    }

    /// Набрать текст и снять снапшот, дождавшись интервала на поддельных часах.
    pub fn type_until_snapshot(session: &Session, file: &Path, chunk: &str) -> usize {
        let mut doc = session.open_file(file).unwrap();
        // Пока часы стоят, снапшот не снимется ни при какой правке.
        doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
        let mut i = 0usize;
        while doc.buffer().snapshots_taken() == 0 && i < 40 {
            let pos = doc.buffer().len();
            doc.insert(pos, chunk.as_bytes()).unwrap();
            doc.snapshot_if_due();
            i += 1;
        }
        // Пять минут прошли — снимок снимает всё, что набрано.
        doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
        doc.snapshot_if_due();
        assert_eq!(doc.buffer().snapshots_taken(), 1, "снапшот не сработал");
        i
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Temp, type_until_snapshot};
    use super::*;
    use crate::api::disk::{KEEP, PERSIST};
    use crate::router::error::SessionError;

    #[test]
    fn snapshot_reaches_disk() {
        // Правки доходят до снапшота на диске — ради этого всё и затевалось.
        let tmp = Temp::new("to-disk");
        let file = tmp.file("a.rs");
        let session = tmp.session(3);

        let typed = type_until_snapshot(&session, &file, "abc");
        assert!(typed > 0);

        let snaps = session.store().list(&file).unwrap();
        assert!(!snaps.is_empty(), "снапшот должен оказаться в базе");
        assert!(snaps[0].text.is_empty(), "исходный текст в базу не пишется");
        let restored = crate::api::buffer::History::decode(&snaps[0].history)
            .unwrap()
            .restore(b"", crate::api::buffer::DEFAULT_DEPTH)
            .unwrap();
        assert_eq!(
            restored.read(),
            "abc".repeat(typed).into_bytes(),
            "набранное восстанавливается из истории"
        );
    }

    #[test]
    fn snapshots_survive_restart() {
        // Главное: второй запуск редактора видит снятое ранее.
        let tmp = Temp::new("restart");
        let file = tmp.file("a.rs");
        let typed = {
            let session = tmp.session(3);
            type_until_snapshot(&session, &file, "xy")
        };
        {
            let session = tmp.session(3);
            let snaps = session.store().list(&file).unwrap();
            assert!(!snaps.is_empty(), "после перезапуска снапшоты должны быть");
            assert!(snaps[0].text.is_empty(), "текст в базе не хранится");
            let restored = crate::api::buffer::History::decode(&snaps[0].history)
                .unwrap()
                .restore(b"", crate::api::buffer::DEFAULT_DEPTH)
                .unwrap();
            assert_eq!(restored.read().len(), typed * 2);
        }
    }

    #[test]
    fn file_text_is_the_source_of_truth() {
        // Снапшот не заменяет файл: документ читается с диска, а не из кеша.
        let tmp = Temp::new("file-truth");
        let file = tmp.file("a.rs");
        std::fs::write(&file, "содержимое файла".as_bytes()).unwrap();

        let session = tmp.session(3);
        let doc = session.open_file(&file).unwrap();
        assert_eq!(doc.text(), "содержимое файла".as_bytes());
        assert_eq!(doc.path(), file.as_path());
    }

    #[test]
    fn missing_file_opens_as_empty() {
        // Файла нет — это новый документ, а не ошибка.
        let tmp = Temp::new("missing");
        let session = tmp.session(3);
        let doc = session.open_file(tmp.file("нет-такого.rs")).unwrap();
        assert!(doc.text().is_empty());
    }

    #[test]
    fn two_documents_write_separate_snapshots() {
        // Два документа в одной сессии не мешают друг другу.
        let tmp = Temp::new("two-docs");
        let a = tmp.file("a.rs");
        let b = tmp.file("b.rs");
        let session = tmp.session(3);

        type_until_snapshot(&session, &a, "a");
        type_until_snapshot(&session, &b, "b");

        let sa = session.store().latest(&a).unwrap().expect("снапшот a");
        let sb = session.store().latest(&b).unwrap().expect("снапшот b");
        assert!(sa.text.is_empty());
        assert!(sb.text.is_empty());
        assert_ne!(sa.history, sb.history, "истории не должны совпасть");
    }

    #[test]
    fn keep_limit_applies_through_session() {
        // Настройка конфига доходит до кольца в базе.
        let tmp = Temp::new("keep");
        let file = tmp.file("a.rs");
        // Короткая история: снапшот по глубине срабатывает каждые несколько
        // правок, и кольцо успевает провернуться.
        let session = tmp.short_session(2, 8);

        let mut doc = session.open_file(&file).unwrap();
        // Часы стоят: по времени снимков не будет, только по глубине.
        doc.set_clock(Box::new(crate::router::clock::FixedClock::after(
            std::time::Duration::ZERO,
        )));
        for _ in 0..40 {
            let end = doc.buffer().len();
            doc.insert(end, b"z").unwrap();
            doc.snapshot_if_due();
        }
        assert!(
            doc.buffer().snapshots_taken() > 2,
            "глубина обязана проворачивать кольцо"
        );
        drop(doc);

        let count = session.store().count(&file).unwrap();
        assert!(
            count <= 2,
            "в базе должно быть не больше keep снапшотов, а их {count}"
        );
    }

    #[test]
    fn unreadable_file_reports_error() {
        // Каталог на месте файла — это не «нет файла», а ошибка чтения.
        let tmp = Temp::new("unreadable");
        let dir = tmp.dir().join("папка");
        std::fs::create_dir_all(&dir).unwrap();
        let session = tmp.session(3);
        let err = session
            .open_file(&dir)
            .expect_err("чтение каталога должно падать");
        match err {
            SessionError::ReadFile { path, .. } => assert!(path.contains("папка")),
            other => panic!("не та ошибка: {other}"),
        }
    }

    #[test]
    fn bad_history_depth_reports_error() {
        // Недопустимая глубина — ошибка, а не паника.
        let tmp = Temp::new("bad-depth");
        let file = tmp.file("a.rs");
        let session = tmp.short_session(3, 0);
        let err = session
            .open_file(&file)
            .expect_err("нулевая глубина недопустима");
        assert!(matches!(err, SessionError::Buffer(_)), "получено: {err}");
    }

    #[test]
    fn session_opens_in_standard_cache_path() {
        // Путь по умолчанию — кеш ОС, а не рабочий каталог.
        let paths = crate::api::disk::StorePaths::from_env();
        let dir = paths.dir.display().to_string();
        assert!(
            dir.ends_with("claxis"),
            "путь должен кончаться на claxis: {dir}"
        );
        assert!(
            dir.contains("cache") || dir.contains("Cache"),
            "это должен быть кеш: {dir}"
        );
    }

    #[test]
    fn persist_false_writes_no_snapshots_to_disk() {
        // Настройка persist обязана что-то значить: при false приёмник не
        // вешается вовсе, поэтому снимок не берётся и на диск ничего не уходит.
        let tmp = Temp::new("no-persist");
        let file = tmp.file("a.rs");
        let session = tmp.no_persist_session(3, 8);

        let mut doc = session.open_file(&file).unwrap();
        doc.set_clock(Box::new(crate::router::clock::FixedClock::after(
            std::time::Duration::ZERO,
        )));
        for _ in 0..40 {
            let end = doc.buffer().len();
            doc.insert(end, b"z").unwrap();
        }
        doc.snapshot();
        // Приёмника нет, значит и снимка нет: история остаётся в памяти.
        assert_eq!(
            doc.buffer().snapshots_taken(),
            0,
            "без приёмника снимок не берётся"
        );
        assert_eq!(doc.buffer().len(), 40, "правки на месте");
        drop(doc);

        // Но на диске пусто.
        assert_eq!(
            session.store().count(&file).unwrap(),
            0,
            "при persist = false на диск не пишется ничего"
        );
    }

    #[test]
    fn defaults_come_from_crate_settings() {
        // Сессия по умолчанию берёт значения из настроек крейтов, а не из
        // зашитых констант.
        let tmp = Temp::new("defaults");
        let session = Session::open_at(tmp.dir().to_path_buf(), KEEP).unwrap();
        assert_eq!(
            session.history_depth(),
            crate::api::buffer::defaults::buffer::HISTORY_DEPTH.value
        );
        assert_eq!(session.persist(), PERSIST);
    }

    #[test]
    fn history_survives_reopening_the_document() {
        // Главное: правки и снапшот переживают перезапуск. Текст верный, но без
        // истории отменять нечего — половина работы пользователя исчезает.
        let tmp = Temp::new("history-survives");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();

        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.delete(0, 1).unwrap();
            assert_eq!(doc.text(), b"bcXY\n");
            // Снапшот уносит историю на диск.
            doc.snapshot();
        }

        // Новый процесс читает тот же файл.
        let session2 = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let mut doc = session2.open_file(&file).unwrap();

        // Текст на месте.
        assert_eq!(doc.text(), b"bcXY\n");
        // И история на месте: отмена работает после перезапуска.
        doc.undo()
            .unwrap()
            .expect("отмена доступна после перезапуска");
        assert_eq!(doc.text(), b"abcXY\n");
    }

    #[test]
    fn a_file_changed_outside_is_not_overwritten_by_the_snapshot() {
        // Файл правили не из редактора. Снапшот построен на прежней версии, и
        // применять его нельзя: правка извне стала бы невидимой и уехала бы под
        // следующим сохранением.
        let tmp = Temp::new("changed-outside");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        // Правка снаружи: и содержимое, и длина.
        std::fs::write(&file, "чужая правка\n").unwrap();

        let session2 = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let doc = session2.open_file(&file).unwrap();
        assert_eq!(
            doc.text(),
            "чужая правка\n".as_bytes(),
            "открыт файл как есть"
        );
        assert!(doc.changed_outside(), "об изменении сообщаем");
    }

    #[test]
    fn a_saved_document_leaves_nothing_behind_on_exit() {
        // После записи работа целиком в файле: и снапшоты, и якорь дальше
        // держать незачем. Выход после сохранения чистит базу.
        let tmp = Temp::new("save-forget");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();

        let written = session.save_document(&doc).unwrap();
        assert!(
            matches!(written, save::Write::Written { .. }),
            "файл под нами не меняли, запись должна пройти: {written:?}"
        );
        session.mark_saved(&mut doc);
        session.forget_snapshots(&file).unwrap();

        let store = session.store().reopen().unwrap();
        assert!(store.latest(&file).unwrap().is_none());
        assert!(store.anchor(&file).unwrap().is_none());
    }

    #[test]
    fn an_unsaved_document_keeps_its_snapshots() {
        // Несохранённая работа — это ещё не отражённая в файле работа: чистить
        // её на выходе значило бы терять то, что человек не успел записать.
        let tmp = Temp::new("save-keep");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();

        let store = session.store().reopen().unwrap();
        assert_eq!(store.count(&file).unwrap(), 1);
    }

    #[test]
    fn a_file_changed_outside_refuses_the_save() {
        // Чужая правка на диске: затирать её отданной работой — врать. Отказ —
        // ответ политики, а не ошибка, и на диск не уезжает ничего.
        let tmp = Temp::new("save-refused");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let mut doc = session.open_file(&file).unwrap();
        std::fs::write(&file, "чужая правка\n").unwrap();
        doc.insert(0, b"XY").unwrap();

        assert_eq!(
            session.save_document(&doc).unwrap(),
            save::Write::Refused(save::Refusal::Changed)
        );
        assert_eq!(
            std::fs::read(&file).unwrap(),
            "чужая правка\n".as_bytes(),
            "на диске стоит чужое"
        );
    }

    #[test]
    fn save_document_reanchors_to_what_was_written() {
        // Якорь всегда о том, что мы последними видели: после записи — о
        // записанном, иначе следующая проверка отвергла бы свой же файл.
        let tmp = Temp::new("save-reanchor");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();
        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let doc = session.open_file(&file).unwrap();

        session.save_document(&doc).unwrap();
        let store = session.store().reopen().unwrap();
        let anchor = store.anchor(&file).unwrap().expect("якорь на месте");
        assert_eq!(anchor.len, 4);
        assert_eq!(anchor.hash, hash::of(b"abc\n"));
    }

    #[test]
    fn keep_zero_means_memory_only_and_writes_nothing() {
        // Ноль в кольце — это «только память». Приёмника быть не должно: иначе
        // он писал бы в базу строку, которую кольцо тут же удаляет, а
        // пользователь с `persist = true` думал бы, что работа переживает
        // перезапуск.
        let tmp = Temp::new("keep-zero");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 0, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let store = session.store().reopen().unwrap();
        assert!(
            store.latest(&file).unwrap().is_none(),
            "в базу не должно уйти ничего"
        );
    }

    #[test]
    fn an_untouched_file_still_gets_its_snapshot() {
        // Обратная сторона проверки: файл не трогали, снапшот применяется.
        let tmp = Temp::new("untouched");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let session2 = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let doc = session2.open_file(&file).unwrap();
        assert_eq!(doc.text(), b"abcXY\n", "история легла поверх файла");
        assert!(!doc.changed_outside(), "файл не меняли, ложной тревоги нет");
    }

    #[test]
    fn a_foreign_history_blob_does_not_break_the_file() {
        // В базе может лежать блоб прежнего буфера: он писал в ту же колонку
        // свой формат. Такой блоб историю не восстановит, но файл обязан
        // открыться, а текст — оказаться на месте.
        let tmp = Temp::new("foreign-blob");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        {
            let mut store = session.store().reopen().unwrap();
            let mut old = b"BUFH".to_vec();
            old.extend_from_slice(&[0u8; 40]);
            store.put(&file, "abc\n".as_bytes(), &old).unwrap();
        }

        let session2 = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        let mut doc = session2.open_file(&file).unwrap();
        assert_eq!(doc.text(), b"abc\n", "текст из снапшота на месте");
        // Истории нет — отменять нечего, и это не ошибка.
        assert!(doc.undo().unwrap().is_none());
        doc.insert(3, b"Z").unwrap();
        assert_eq!(doc.text(), b"abcZ\n", "документ рабочий");
    }

    #[test]
    fn a_rope_snapshot_is_written_in_the_rope_format() {
        // Что уходит в базу, обязан быть форматом rope: с магией и версией.
        // Иначе следующий запуск не поймёт, что там лежит.
        let tmp = Temp::new("rope-format");
        let file = tmp.dir().join("doc.txt");
        std::fs::write(&file, "abc\n").unwrap();

        let session = Session::open_at_with(tmp.dir().to_path_buf(), 3, true, 64).unwrap();
        {
            let mut doc = session.open_file(&file).unwrap();
            doc.insert(3, b"XY").unwrap();
            doc.snapshot();
        }

        let store = session.store().reopen().unwrap();
        let snapshot = store.latest(&file).unwrap().expect("снапшот записан");
        assert_eq!(&snapshot.history[..4], b"CLRP", "магия формата rope");
        assert!(snapshot.history.len() > 5, "в блобе есть версия");

        // И разбирается тем же кодом, без потерь.
        let restored = crate::api::buffer::History::decode(&snapshot.history)
            .expect("блоб из базы — наш")
            .restore(b"abc\n", crate::api::buffer::DEFAULT_DEPTH)
            .expect("документ собирается");
        assert_eq!(restored.read(), b"abcXY\n");
    }
}

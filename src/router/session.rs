//! Связывает документ с хранилищем снапшотов.
//!
//! Оба крейта по отдельности ничего не знают друг о друге: `claxis-rope`
//! отдаёт текст и историю приёмнику и не знает, куда они уйдут,
//! `claxis-store` умеет только складывать и доставать байты. Соединяются они
//! здесь — это и есть работа маршрутизатора.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api::rope::defaults::buffer::HISTORY_DEPTH;
use crate::api::rope::{Document as Rope, Error as RopeError, History, SnapshotSink};
use crate::api::store::{KEEP, PERSIST, SnapshotStore, StorePaths};
use crate::messages::Messages;
use crate::router::stats::{self, Stats};

/// Что пошло не так в сессии.
#[derive(Debug)]
pub enum SessionError {
    /// Документ не создан: глубина истории недопустима.
    Buffer(crate::api::rope::Error),
    /// Хранилище не открылось.
    Store { path: String, reason: String },
    /// Файл документа не прочитан.
    ReadFile { path: String, reason: String },
    /// Файл больше, чем помещается в `u32`.
    FileTooLarge { bytes: u64 },
}

impl SessionError {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            SessionError::Buffer(e) => messages.buffer_error(e),
            SessionError::Store { path, reason } => messages.store_failed(path, reason),
            SessionError::ReadFile { path, reason } => messages.read_file_failed(path, reason),
            SessionError::FileTooLarge { bytes } => messages.file_too_large(
                *bytes,
                *bytes as f64 / 1_000_000_000.0,
                u32::MAX as u64,
                u32::MAX as f64 / 1_000_000_000.0,
            ),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for SessionError {}

/// Приёмник снапшота, пишущий в хранилище.
///
/// Это тот самый стык: документ отдаёт текст и историю, дальше они уходят в
/// sqlite байтами, в том формате, в котором их собрал rope.
#[derive(Debug)]
struct StoreSink {
    store: SnapshotStore,
}

impl SnapshotSink for StoreSink {
    fn accept(&mut self, file: &Path, text: &[u8], history: &[u8]) {
        // Ошибка сохранения снапшота не ломает правку: потеря кеша не должна
        // стоить пользователю работы.
        //
        // История идёт вместе с текстом: без неё после перезапуска отменять
        // было бы нечего, то есть снапшот сохранил бы половину работы.
        let _ = self.store.put(file, text, history);
    }
}

/// Источник времени.
///
/// Отдельный тип, а не `Instant::now()` в коде: иначе проверить, что сработало
/// правильно, можно было бы только ожиданием вживую.
pub trait Clock: std::fmt::Debug {
    /// Сколько прошло с начала работы процесса.
    fn elapsed(&self) -> Duration;
}

/// Часы, которые двигает тест: ждать настоящие пять минут нельзя.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub Duration);

impl FixedClock {
    /// Часы, стоящие на указанной отметке.
    pub fn at(at: Duration) -> Self {
        Self(at)
    }
}

impl Clock for FixedClock {
    fn elapsed(&self) -> Duration {
        self.0
    }
}

/// Настоящие часы.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
}

/// Открытый документ: rope плюс привязка к файлу.
///
/// Время живёт здесь, а не в документе: снапшот — это решение сессии, у неё
/// есть и документ, и хранилище. Сам документ про часы не знает и снимает
/// снапшот только по прямой просьбе.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    buffer: Rope,
    /// Через сколько снимать следующий снапшот.
    interval: Duration,
    /// Сколько прошло с начала работы, когда снят последний снапшот.
    last_snapshot: Duration,
    /// Откуда берётся время.
    clock: Box<dyn Clock>,
    /// Что набрано за сеанс работы над этим документом.
    stats: Stats,
}

impl Document {
    /// Путь к файлу.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Документ: правки, чтение и история.
    pub fn buffer(&mut self) -> &mut Rope {
        &mut self.buffer
    }

    /// Текст документа.
    pub fn text(&self) -> Vec<u8> {
        self.buffer.read()
    }

    /// Что собрано за сеанс: символы, нажатия, правки.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Вставить текст и посчитать введённые символы.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) -> Result<(), RopeError> {
        self.buffer.insert(anchor, data)?;
        stats::note_input(&mut self.stats, data);
        Ok(())
    }

    /// Вырезать диапазон и посчитать удалённые символы.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<(), RopeError> {
        self.buffer.delete(anchor, len)?;
        stats::note_deleted(&mut self.stats, len);
        Ok(())
    }

    /// Заменить диапазон: вставка и удаление одной операцией.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<(), RopeError> {
        self.buffer.replace(anchor, len, data)?;
        stats::note_replace(&mut self.stats, data, len);
        Ok(())
    }

    /// Записать одно нажатие вместе со временем, которое оно заняло.
    ///
    /// Нажатие без отпускания — обычное дело: палец на клавише или потерянное
    /// событие. Такое нажатие засчитывается, но времени не добавляет.
    pub fn note_press(&mut self, held: Duration) {
        stats::note_press(&mut self.stats, held);
    }

    /// Через сколько после последнего снапшота снимать следующий.
    pub fn snapshot_interval(&self) -> Duration {
        self.interval
    }

    /// Подменить часы: тестам нужно время, которое они двигают сами.
    ///
    /// Отсчёт не сбрасывается: иначе перевести часы вперёд было бы нельзя —
    /// документ решил бы, что только что снял снапшот.
    pub fn set_clock(&mut self, clock: Box<dyn Clock>) {
        self.clock = clock;
    }

    /// Снять снапшот, если с последнего прошло достаточно времени.
    ///
    /// Время, а не число правок: человек может править один и тот же абзац
    /// десять минут, и снапшот каждые пять минут отдаст память обратно. И
    /// наоборот — сто правок подряд за минуту снапшот не заслужили.
    pub fn snapshot_if_due(&mut self) {
        if self.clock.elapsed().saturating_sub(self.last_snapshot) >= self.interval {
            self.snapshot();
        }
    }

    /// Снять снапшот и отсчитать время заново.
    pub fn snapshot(&mut self) {
        self.buffer.snapshot();
        self.last_snapshot = self.clock.elapsed();
    }
}

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
    /// Глубина, с которой сессия открыта и под которую созданы буферы.
    ///
    /// Отличается от `history_depth`, если конфиг поменяли на лету: буферы
    /// уже созданы под прежнюю, и пересоздать их без потери истории нельзя.
    opened_depth: u32,
}

/// Что применилось сразу, а что ждёт перезапуска.
///
/// Не всё настраивается на лету: размер пула узлов задан при создании буфера,
/// и сменить его можно только пересоздав буфер вместе с историей. Лучше сказать
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
        dir: &Path,
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
    /// открытии. `history_depth` задаёт размер пула узлов при создании буфера
    /// и без пересоздания истории не меняется.
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

        applied
    }

    /// Открыть документ: прочитать файл и привязать снапшоты к хранилищу.
    ///
    /// Файла может не быть — тогда это новый пустой документ, ошибка не
    /// возникает.
    pub fn open_file(&self, path: impl AsRef<Path>) -> Result<Document, SessionError> {
        let path = path.as_ref().to_path_buf();

        // Проверяем размер до чтения: буфер работает с u32, файл больше
        // u32::MAX байт не помещается.
        if let Ok(metadata) = std::fs::metadata(&path) {
            let len = metadata.len();
            if len > u32::MAX as u64 {
                return Err(SessionError::FileTooLarge { bytes: len });
            }
        }

        let text = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(SessionError::ReadFile {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                });
            }
        };
        let mut buffer =
            Rope::with_depth(text, self.history_depth).map_err(SessionError::Buffer)?;
        // Снапшот под документом копит не только текст, но и историю правок.
        // Восстанавливаем её: после перезапуска отменять должно быть можно,
        // иначе половина работы пользователя просто исчезает.
        if self.persist
            && let Ok(store) = self.store.reopen()
            && let Ok(Some(snapshot)) = store.latest(&path)
        {
            // Чужой или битый блоб историю не восстановит — и не должен:
            // текст-то из базы верный, терять его из-за испорченного хвоста
            // незачем.
            if let Ok(restored) = History::decode(&snapshot.history).and_then(|h| h.restore()) {
                buffer = restored;
            }
        }
        // Приёмник вешается только если снапшоты положено хранить. Иначе
        // буфер работает без диска и настройка `persist` что-то значит.
        if self.persist {
            // Своё подключение к той же базе: буфер живёт дольше сессии.
            let store = self.store.reopen().map_err(|e| SessionError::Store {
                path: self.paths.database().display().to_string(),
                reason: e.to_string(),
            })?;
            buffer.set_sink(path.clone(), Box::new(StoreSink { store }));
        }
        Ok(Document {
            path,
            buffer,
            interval: self.interval,
            last_snapshot: Duration::ZERO,
            clock: Box::new(SystemClock),
            stats: Stats::default(),
        })
    }
}

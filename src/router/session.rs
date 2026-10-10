//! Связывает буфер с хранилищем снапшотов.
//!
//! Оба крейта по отдельности ничего не знают друг о друге: `claxis-buffer`
//! отдаёт текст приёмнику и не знает, куда он уйдёт, `claxis-store` умеет
//! только складывать и доставать. Соединяются они здесь — это и есть работа
//! маршрутизатора.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::api::buffer::{Buffer, HISTORY_DEPTH, SnapshotSink};
use crate::api::store::{KEEP, PERSIST, SnapshotStore, StorePaths};
use crate::messages::Messages;

/// Что пошло не так в сессии.
#[derive(Debug)]
pub enum SessionError {
    /// Буфер не создан: глубина истории недопустима.
    Buffer(claxis_buffer::Error),
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
/// Это тот самый стык: буфер отдаёт текст, дальше он уходит в sqlite.
#[derive(Debug)]
struct StoreSink {
    store: SnapshotStore,
}

impl SnapshotSink for StoreSink {
    fn accept(&mut self, file: &Path, text: &[u8]) {
        // Ошибка сохранения снапшота не ломает правку: потеря кеша не должна
        // стоить пользователю работы.
        let _ = self.store.put(file, text);
    }
}

/// Открытый документ: буфер плюс привязка к файлу.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    buffer: Buffer,
}

impl Document {
    /// Путь к файлу.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Буфер документа.
    pub fn buffer(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// Текст документа.
    pub fn text(&self) -> Vec<u8> {
        self.buffer.read()
    }
}

/// Редакторская сессия: хранилище снапшотов и открытые документы.
#[derive(Debug)]
pub struct Session {
    store: SnapshotStore,
    paths: StorePaths,
    /// Сколько записей правки живёт в памяти до снапшота. Из конфига.
    history_depth: u32,
    /// Писать ли снапшоты на диск. Из конфига.
    persist: bool,
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
    pub fn open(keep: u32, persist: bool, history_depth: u32) -> Result<Self, SessionError> {
        let paths = StorePaths::from_env();
        Self::build(paths, keep, persist, history_depth)
    }

    fn build(
        paths: StorePaths,
        keep: u32,
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
            opened_depth: history_depth,
        })
    }

    /// Открыть сессию с хранилищем в заданном каталоге — нужно в тестах.
    pub fn open_at(dir: impl Into<PathBuf>, keep: u32) -> Result<Self, SessionError> {
        Self::open_at_with(dir, keep, PERSIST, HISTORY_DEPTH.value)
    }

    /// То же, но с явными `persist` и глубиной истории.
    pub fn open_at_with(
        dir: impl Into<PathBuf>,
        keep: u32,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::build(StorePaths::new(dir), keep, persist, history_depth)
    }

    /// То же, но путь принимается как ссылка.
    pub fn open_at_ref(
        dir: &Path,
        keep: u32,
        persist: bool,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::open_at_with(dir.to_path_buf(), keep, persist, history_depth)
    }

    /// Сколько снапшотов одного файла хранится.
    pub fn keep(&self) -> u32 {
        self.store.keep()
    }

    /// Пишутся ли снапшоты на диск.
    pub fn persist(&self) -> bool {
        self.persist
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
            Buffer::with_history_depth(text, self.history_depth).map_err(SessionError::Buffer)?;
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
        Ok(Document { path, buffer })
    }
}

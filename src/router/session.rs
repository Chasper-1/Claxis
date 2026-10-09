//! Связывает буфер с хранилищем снапшотов.
//!
//! Оба крейта по отдельности ничего не знают друг о друге: `claxis-buffer`
//! отдаёт текст приёмнику и не знает, куда он уйдёт, `claxis-store` умеет
//! только складывать и доставать. Соединяются они здесь — это и есть работа
//! маршрутизатора.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::api::buffer::{Buffer, DEFAULT_DEPTH, SnapshotSink};
use crate::api::store::{SnapshotStore, StorePaths};

/// Что пошло не так в сессии.
#[derive(Debug)]
pub enum SessionError {
    /// Буфер не создан: глубина истории недопустима.
    Buffer(claxis_buffer::Error),
    /// Хранилище не открылось.
    Store { path: String, reason: String },
    /// Файл документа не прочитан.
    ReadFile { path: String, reason: String },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Buffer(e) => write!(f, "{e}"),
            SessionError::Store { path, reason } => {
                write!(f, "cannot open store at {path}: {reason}")
            }
            SessionError::ReadFile { path, reason } => {
                write!(f, "cannot read file {path}: {reason}")
            }
        }
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
}

impl Session {
    /// Открыть сессию с хранилищем в кеше, `keep` снапшотов на файл.
    pub fn open(keep: u32) -> Result<Self, SessionError> {
        Self::open_with(keep, DEFAULT_DEPTH)
    }

    /// Открыть сессию с заданными `keep` и глубиной истории.
    pub fn open_with(keep: u32, history_depth: u32) -> Result<Self, SessionError> {
        let paths = StorePaths::from_env();
        Self::build(paths, keep, history_depth)
    }

    fn build(paths: StorePaths, keep: u32, history_depth: u32) -> Result<Self, SessionError> {
        let store = SnapshotStore::open(&paths, keep).map_err(|e| SessionError::Store {
            path: paths.database().display().to_string(),
            reason: e.to_string(),
        })?;
        Ok(Self {
            store,
            paths,
            history_depth,
        })
    }

    /// Открыть сессию с хранилищем в заданном каталоге — нужно в тестах.
    pub fn open_at(dir: impl Into<PathBuf>, keep: u32) -> Result<Self, SessionError> {
        Self::open_at_with(dir, keep, DEFAULT_DEPTH)
    }

    /// То же, но с заданной глубиной истории.
    pub fn open_at_with(
        dir: impl Into<PathBuf>,
        keep: u32,
        history_depth: u32,
    ) -> Result<Self, SessionError> {
        Self::build(StorePaths::new(dir), keep, history_depth)
    }

    /// Глубина истории, с которой открыты документы.
    pub fn history_depth(&self) -> u32 {
        self.history_depth
    }

    /// Где лежит хранилище.
    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// Хранилище снапшотов.
    pub fn store(&self) -> &SnapshotStore {
        &self.store
    }

    /// Открыть документ: прочитать файл и привязать снапшоты к хранилищу.
    ///
    /// Файла может не быть — тогда это новый пустой документ, ошибка не
    /// возникает.
    pub fn open_file(&self, path: impl AsRef<Path>) -> Result<Document, SessionError> {
        let path = path.as_ref().to_path_buf();
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
        // Своё подключение к той же базе: буфер живёт дольше сессии.
        let store = self.store.reopen().map_err(|e| SessionError::Store {
            path: self.paths.database().display().to_string(),
            reason: e.to_string(),
        })?;
        buffer.set_sink(path.clone(), Box::new(StoreSink { store }));
        Ok(Document { path, buffer })
    }
}

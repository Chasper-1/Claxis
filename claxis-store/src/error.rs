use std::fmt;

use crate::messages::Messages;

/// Что пошло не так в хранилище.
#[derive(Debug)]
pub enum Error {
    /// Файл базы не открылся.
    Open { path: String, reason: String },
    /// Каталог для базы не создался.
    PrepareDir { path: String, reason: String },
    /// Запрос к базе не выполнился.
    Query { reason: String },
    /// Снапшота для файла нет.
    NoSnapshot { file: String },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::Open { path, reason } => messages.open_failed(path, reason),
            Error::PrepareDir { path, reason } => messages.prepare_dir_failed(path, reason),
            Error::Query { reason } => messages.query_failed(reason),
            Error::NoSnapshot { file } => messages.no_snapshot(file),
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

/// Результат операции хранилища.
pub type Result<T> = std::result::Result<T, Error>;

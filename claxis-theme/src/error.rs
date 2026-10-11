//! Что не так с темой.

use std::fmt;

use crate::messages::Messages;

/// Ошибка темы.
#[derive(Debug)]
pub enum Error {
    /// Темы с таким именем нет: `{name}`. Рядом — что есть: `{available}`.
    UnknownTheme {
        /// Имя, которое запросили.
        name: String,
        /// Темы, которые нашлись.
        available: Vec<String>,
    },
    /// Папка с темами не прочитана: `{dir}`, `{reason}`.
    ReadDir {
        /// Путь к папке.
        dir: String,
        /// Почему не прочитана.
        reason: String,
    },
    /// Имя темы не годится: `{name}`.
    InvalidName {
        /// Что пришло в качестве имени.
        name: String,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::UnknownTheme { name, available } => messages.unknown_theme(name, available),
            Error::ReadDir { dir, reason } => messages.read_dir_failed(dir, reason),
            Error::InvalidName { name } => messages.invalid_theme_name(name),
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

/// Результат операции с темой.
pub type Result<T> = std::result::Result<T, Error>;

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    crate::messages::samples()
}

//! Что пошло не так в конфиге.

use std::fmt;

/// Ошибка конфига.
#[derive(Debug)]
pub enum Error {
    /// Файл не прочитан: `{path}`, `{reason}`.
    Read {
        /// Путь к файлу.
        path: String,
        /// Почему не прочитан.
        reason: String,
    },
    /// Конфиг разобран неправильно: `{path}`, `{reason}`.
    Parse {
        /// Путь к файлу.
        path: String,
        /// Что не так.
        reason: String,
    },
    /// Ключ `{key}` не существует.
    ///
    /// Молчаливый пропуск означал бы, что опечатку в настройке никто не
    /// заметит, а редактор поведёт себя не так, как ожидают.
    UnknownKey {
        /// Какой ключ не признан.
        key: String,
    },
    /// Ключ `{key}` есть, но значение `{got}` не подходит: ожидалось {expected}.
    BadValue {
        /// Какой ключ.
        key: String,
        /// Что пришло.
        got: String,
        /// Что ожидалось.
        expected: &'static str,
    },
    /// Ключ задан не в своей секции: `{key}` ожидался в `{want}`, а стоит в `{got}`.
    WrongSection {
        /// Какой ключ.
        key: String,
        /// Где он должен быть.
        want: String,
        /// Где он оказался.
        got: String,
    },
    /// Нет сохранённого правильного конфига, чтобы продолжить работу.
    NoLastGood,
    /// В файле `files.toml` ключ `{key}`, которого нет в схеме.
    UnknownKeyInFile {
        /// Имя файла.
        file: &'static str,
        /// Какой ключ не признан.
        key: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Read { path, reason } => write!(f, "cannot read config {path}: {reason}"),
            Error::Parse { path, reason } => write!(f, "cannot parse config {path}: {reason}"),
            Error::UnknownKey { key } => write!(f, "unknown key: {key}"),
            Error::BadValue { key, got, expected } => {
                write!(f, "bad value for {key}: {got}, expected {expected}")
            }
            Error::WrongSection { key, want, got } => {
                write!(f, "key {key} belongs to {want}, but is in {got}")
            }
            Error::NoLastGood => write!(f, "no saved valid config to fall back to"),
            Error::UnknownKeyInFile { file, key } => {
                write!(f, "{file}: unknown key {key}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Результат операции с конфигом.
pub type Result<T> = std::result::Result<T, Error>;

//! Что пошло не так в конфиге.

use std::fmt;

use crate::messages::Messages;

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
    /// Файл не записан: `{path}`, `{reason}`.
    Write {
        /// Путь к файлу.
        path: String,
        /// Почему не записан.
        reason: String,
    },
    /// В файле `files.toml` ключ `{key}`, которого нет в схеме.
    UnknownKeyInFile {
        /// Имя файла.
        file: &'static str,
        /// Какой ключ не признан.
        key: String,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, m: &dyn Messages) -> String {
        match self {
            Error::Read { path, reason } => m.read_failed(path, reason),
            Error::Write { path, reason } => m.write_failed(path, reason),
            Error::Parse { path, reason } => m.parse_failed(path, reason),
            Error::UnknownKey { key } => m.unknown_key(key),
            Error::BadValue { key, got, expected } => m.bad_value(key, got, expected),
            Error::WrongSection { key, want, got } => m.wrong_section(key, want, got),
            Error::NoLastGood => m.no_last_good(),
            Error::UnknownKeyInFile { file, key } => m.unknown_key_in_file(file, key),
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

/// Результат операции с конфигом.
pub type Result<T> = std::result::Result<T, Error>;

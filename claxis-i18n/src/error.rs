//! Что пошло не так в переводе.

use std::fmt;

/// Ошибка перевода.
#[derive(Debug)]
pub enum Error {
    /// Языка нет: `{lang}`.
    UnknownLang {
        /// Код языка, которого не нашлось.
        lang: String,
    },
    /// Файл перевода не прочитан: `{path}`, `{reason}`.
    Read {
        /// Путь к файлу.
        path: String,
        /// Почему не прочитан.
        reason: String,
    },
    /// Файл перевода разобран неправильно: `{path}`, `{reason}`.
    Parse {
        /// Путь к файлу.
        path: String,
        /// Что не так.
        reason: String,
    },
    /// Перевод неполный: не хватает `{count}` строк из `{total}`.
    Incomplete {
        /// Код языка.
        lang: String,
        /// Сколько строк не хватает.
        count: usize,
        /// Сколько всего строк должно быть.
        total: usize,
    },
    /// Ключ повторяется дважды.
    DuplicateKey {
        /// Код языка.
        lang: String,
        /// Какой ключ повторился.
        key: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownLang { lang } => write!(f, "unknown language: {lang}"),
            Error::Read { path, reason } => write!(f, "cannot read translation {path}: {reason}"),
            Error::Parse { path, reason } => {
                write!(f, "cannot parse translation {path}: {reason}")
            }
            Error::Incomplete { lang, count, total } => write!(
                f,
                "translation for {lang} is incomplete: {count} of {total} strings missing"
            ),
            Error::DuplicateKey { lang, key } => {
                write!(f, "translation for {lang} repeats the key {key}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Результат операции с переводом.
pub type Result<T> = std::result::Result<T, Error>;

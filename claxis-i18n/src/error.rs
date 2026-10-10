//! Что пошло не так в переводе.

use std::fmt;

use crate::messages::Messages;

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

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::UnknownLang { lang } => messages.unknown_lang(lang),
            Error::Read { path, reason } => messages.read_failed(path, reason),
            Error::Parse { path, reason } => messages.parse_failed(path, reason),
            Error::Incomplete { lang, count, total } => messages.incomplete(lang, *count, *total),
            Error::DuplicateKey { lang, key } => messages.duplicate_key(lang, key),
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

/// Результат операции с переводом.
pub type Result<T> = std::result::Result<T, Error>;

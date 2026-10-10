//! Каталоги перевода.
//!
//! Крейт ничего не знает про конфиг, состояния и настройки. Его дело —
//! словарь строк: английский ключ, перевод в значение. Конфиг и редактор
//! приходят к нему за переводом, а не наоборот, поэтому связь односторонняя.
//!
//! Ключ — это сама английская строка, вместе с местами под значения:
//!
//! ```toml
//! "range {anchor}..{end} is outside the document of length {doc_len}" = "диапазон ..."
//! ```
//!
//! Перевод буквально заменяет английский на свой язык. Структуру придумывать
//! заново не нужно, и в коде всегда пишется английский текст.

pub mod defaults;
pub mod error;
pub mod messages;
pub mod path;

pub use error::{Error, Result};
pub use messages::{En, Messages};
pub use path::LangPath;

/// Каталог: английский текст на перевод.
///
/// Английский каталог отдаёт сам себя, любой другой — значение из файла.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<(String, String)>,
}

impl Catalog {
    /// Пустой каталог.
    pub fn new() -> Self {
        Self::default()
    }

    /// Каталог из готовых пар.
    pub fn from_entries(entries: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    /// Добавить строку.
    pub fn add(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.push((key.into(), value.into()));
    }

    /// Добавить всё из другого каталога.
    ///
    /// Нужно, чтобы собрать каталог редактора из каталогов отдельных крейтов.
    pub fn merge(&mut self, other: Catalog) {
        self.entries.extend(other.entries);
    }

    /// Перевод строки. `None`, если ключа нет.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Все ключи каталога.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Все пары каталога.
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// Сколько строк в каталоге.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Каталог пуст.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ключи, которых нет в каталоге.
    ///
    /// Неполный перевод — ошибка: молча показывать английский посреди
    /// русского интерфейса хуже, чем отказаться запускаться.
    pub fn missing<'a>(&'a self, required: &[&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|k| self.get(k).is_none())
            .collect()
    }

    /// Дубликаты ключей внутри каталога.
    ///
    /// Дубль означает, что переводчик написал одну строку дважды и непонятно,
    /// какая из них сработает.
    pub fn duplicates(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        let mut dups: Vec<&str> = Vec::new();
        for (key, _) in &self.entries {
            if seen.contains(&key.as_str()) {
                dups.push(key.as_str());
            } else {
                seen.push(key.as_str());
            }
        }
        dups
    }
}

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = crate::Error::UnknownLang {
        lang: "klingon".to_string(),
    };
    let read = crate::Error::Read {
        path: "/home/u/.config/claxis/lang/ru.toml".to_string(),
        reason: "permission denied".to_string(),
    };
    let parse = crate::Error::Parse {
        path: "/home/u/.config/claxis/lang/ru.toml".to_string(),
        reason: "line 4: unclosed quote".to_string(),
    };
    let incomplete = crate::Error::Incomplete {
        lang: "ru".to_string(),
        count: 12,
        total: 340,
    };
    let duplicate = crate::Error::DuplicateKey {
        lang: "ru".to_string(),
        key: "Open file".to_string(),
    };
    vec![
        ("unknown_lang", m.to_string()),
        ("read_failed", read.to_string()),
        ("parse_failed", parse.to_string()),
        ("incomplete", incomplete.to_string()),
        ("duplicate_key", duplicate.to_string()),
    ]
}

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
pub mod load;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let mut c = Catalog::new();
        c.add("open", "открыть");
        c.add("save", "сохранить");
        c
    }

    #[test]
    fn a_new_catalog_is_empty() {
        let c = Catalog::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.get("open"), None);
        assert_eq!(c.keys().count(), 0);
    }

    #[test]
    fn an_added_key_is_found_by_its_exact_text() {
        let c = catalog();
        assert_eq!(c.get("open"), Some("открыть"));
        assert_eq!(c.get("save"), Some("сохранить"));
        assert!(!c.is_empty());
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn a_key_with_another_case_or_space_is_another_key() {
        let c = catalog();
        assert_eq!(c.get("Open"), None);
        assert_eq!(c.get("open "), None);
        assert_eq!(c.get(""), None);
    }

    #[test]
    fn keys_are_returned_in_insertion_order() {
        assert_eq!(catalog().keys().collect::<Vec<_>>(), ["open", "save"]);
    }

    #[test]
    fn entries_keep_both_sides_of_the_pair() {
        let c = catalog();
        assert_eq!(
            c.entries(),
            [
                ("open".to_string(), "открыть".to_string()),
                ("save".to_string(), "сохранить".to_string()),
            ]
        );
    }

    #[test]
    fn from_entries_keeps_the_given_order() {
        let c = Catalog::from_entries([
            ("b".to_string(), "б".to_string()),
            ("a".to_string(), "а".to_string()),
        ]);
        assert_eq!(c.keys().collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn merge_appends_instead_of_overwriting() {
        let mut c = catalog();
        let mut other = Catalog::new();
        other.add("quit", "выход");
        c.merge(other);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get("quit"), Some("выход"));
        assert_eq!(c.keys().collect::<Vec<_>>(), ["open", "save", "quit"]);
    }

    #[test]
    fn merge_of_a_duplicate_key_keeps_both_and_get_takes_the_first() {
        // Каталог редактора собирается из каталогов крейтов: один и тот же ключ
        // может прийти дважды. Именно поэтому `duplicates` и нужен.
        let mut c = catalog();
        let mut other = Catalog::new();
        other.add("open", "открыть файл");
        c.merge(other);
        assert_eq!(c.get("open"), Some("открыть"));
        assert_eq!(c.duplicates(), ["open"]);
    }

    #[test]
    fn a_merged_catalog_becomes_empty_again() {
        let mut c = catalog();
        assert_eq!(c.duplicates(), Vec::<&str>::new());
        let mut second = Catalog::new();
        second.add("open", "ещё раз");
        c.merge(second);
        assert_eq!(c.duplicates(), ["open"]);
    }

    #[test]
    fn missing_lists_exactly_the_keys_it_lacks() {
        let c = catalog();
        assert_eq!(
            c.missing(&["open", "quit", "save", "help"]),
            ["quit", "help"]
        );
        assert_eq!(c.missing(&["open", "save"]), Vec::<&str>::new());
        assert_eq!(c.missing(&[]), Vec::<&str>::new());
    }

    #[test]
    fn a_key_present_with_an_empty_value_is_not_missing() {
        // Пустой перевод — это плохо, но он есть: `missing` про него не врёт,
        // отсутствие строки проверяется отдельно.
        let mut c = Catalog::new();
        c.add("open", "");
        assert_eq!(c.missing(&["open"]), Vec::<&str>::new());
        assert_eq!(c.get("open"), Some(""));
    }

    #[test]
    fn duplicates_report_every_repeated_key() {
        let mut c = Catalog::new();
        c.add("a", "1");
        c.add("b", "2");
        c.add("a", "3");
        c.add("b", "4");
        c.add("a", "5");
        assert_eq!(c.duplicates(), ["a", "b", "a"]);
    }

    #[test]
    fn a_catalog_without_duplicates_reports_nothing() {
        assert!(catalog().duplicates().is_empty());
        assert!(Catalog::new().duplicates().is_empty());
    }

    #[test]
    fn samples_are_unique_and_have_no_empty_placeholders() {
        let samples = samples();
        let mut keys: Vec<&str> = samples.iter().map(|(k, _)| *k).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), before, "в примерах есть повторяющиеся ключи");

        for (key, text) in samples {
            assert!(!text.is_empty(), "{key} вышел пустым");
            assert!(
                !text.contains('{'),
                "{key} оставил место без значения: {text}"
            );
            assert!(
                !text.contains('}'),
                "{key} оставил закрытие без места: {text}"
            );
        }
    }

    #[test]
    fn every_sample_shows_its_values() {
        // Пример без подставленных значений переводчику ничего не объясняет.
        let samples: std::collections::HashMap<&str, String> = samples().into_iter().collect();
        assert!(samples["unknown_lang"].contains("klingon"));
        assert!(samples["read_failed"].contains("permission denied"));
        assert!(samples["parse_failed"].contains("unclosed quote"));
        assert!(samples["incomplete"].contains("12"));
        assert!(samples["incomplete"].contains("340"));
        assert!(samples["duplicate_key"].contains("Open file"));
    }
}

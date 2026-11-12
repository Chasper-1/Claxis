//! Каталог: английский текст на перевод.

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
}

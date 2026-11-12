//! Сообщения об ошибках перевода.
//!
//! Перевод не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use claxis_text::substitute;

/// Каталог сообщений об ошибках перевода.
///
/// Места под значения: `{lang}`, `{path}`, `{reason}`, `{count}`, `{total}`,
/// `{key}`.
pub trait Messages {
    /// Языка нет.
    fn unknown_lang(&self, lang: &str) -> String;
    /// Файл перевода не прочитан.
    fn read_failed(&self, path: &str, reason: &str) -> String;
    /// Файл перевода разобран неправильно.
    fn parse_failed(&self, path: &str, reason: &str) -> String;
    /// Перевод неполный.
    fn incomplete(&self, lang: &str, count: usize, total: usize) -> String;
    /// Ключ повторяется дважды.
    fn duplicate_key(&self, lang: &str, key: &str) -> String;
    /// Значение перевода оказалось не строкой: `{path}`, `{key}`.
    fn not_a_string(&self, path: &str, key: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn unknown_lang(&self, lang: &str) -> String {
        substitute("unknown language: {lang}", &[("lang", lang.to_string())])
    }

    fn read_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot read translation {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn parse_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot parse translation {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn incomplete(&self, lang: &str, count: usize, total: usize) -> String {
        substitute(
            "translation for {lang} is incomplete: {count} of {total} strings missing",
            &[
                ("lang", lang.to_string()),
                ("count", count.to_string()),
                ("total", total.to_string()),
            ],
        )
    }

    fn duplicate_key(&self, lang: &str, key: &str) -> String {
        substitute(
            "translation for {lang} repeats the key {key}",
            &[("lang", lang.to_string()), ("key", key.to_string())],
        )
    }

    fn not_a_string(&self, path: &str, key: &str) -> String {
        substitute(
            "translation {path}: value of {key} is not a string",
            &[("path", path.to_string()), ("key", key.to_string())],
        )
    }
}
/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        ("unknown_lang", m.unknown_lang("klingon")),
        (
            "read_failed",
            m.read_failed("/home/u/.config/claxis/lang/ru.toml", "permission denied"),
        ),
        (
            "parse_failed",
            m.parse_failed(
                "/home/u/.config/claxis/lang/ru.toml",
                "line 4: unclosed quote",
            ),
        ),
        ("incomplete", m.incomplete("ru", 12, 340)),
        ("duplicate_key", m.duplicate_key("ru", "Open file")),
        (
            "not_a_string",
            m.not_a_string("/home/u/.config/claxis/lang/ru.toml", "Open file"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::samples;

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
        assert!(samples["not_a_string"].contains("Open file"));
    }
}

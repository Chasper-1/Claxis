//! Где лежат переводы.

use std::path::{Path, PathBuf};

use crate::defaults::{DEFAULT_LANGUAGE, LANGUAGE_DIR};

/// Путь к файлу перевода.
///
/// Название файла — код языка: `ru.toml`. На генерацию шаблона название не
/// влияет: шаблон строится из кода, а не из файлов в каталоге.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LangPath(PathBuf);

impl LangPath {
    /// Файл перевода для языка `lang` в каталоге `dir`.
    ///
    /// `lang` проверяется: путь вида `../../etc/passwd` не должен уводить за
    /// пределы каталога переводов.
    pub fn new(dir: impl AsRef<Path>, lang: &str) -> Result<Self, super::Error> {
        if !Self::is_valid_code(lang) {
            return Err(super::Error::UnknownLang {
                lang: lang.to_string(),
            });
        }
        Ok(Self(dir.as_ref().join(format!("{lang}.toml"))))
    }

    /// Код языка выглядит как `ru` или `pt-br`: буквы, цифры, один дефис.
    ///
    /// Без этой проверки путь можно увести вверх и прочитать чужой файл.
    pub fn is_valid_code(lang: &str) -> bool {
        if lang.is_empty() || lang.len() > 16 {
            return false;
        }
        let mut parts = lang.split('-');
        let Some(primary) = parts.next() else {
            return false;
        };
        if primary.is_empty() || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
            return false;
        }
        for part in parts {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
        }
        true
    }

    /// Сам файл.
    pub fn file(&self) -> &Path {
        &self.0
    }

    /// Каталог, в котором лежат переводы.
    pub fn dir(&self) -> &Path {
        self.0.parent().unwrap_or_else(|| Path::new("."))
    }
}

/// Каталог переводов по умолчанию: `<конфиг>/lang`.
pub fn default_dir(config_dir: impl AsRef<Path>) -> PathBuf {
    config_dir.as_ref().join(LANGUAGE_DIR)
}

/// Язык по умолчанию.
pub fn default_language() -> &'static str {
    DEFAULT_LANGUAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_codes_are_valid() {
        assert!(LangPath::is_valid_code("ru"));
        assert!(LangPath::is_valid_code("en"));
        assert!(LangPath::is_valid_code("pt"));
    }

    #[test]
    fn regional_codes_are_valid() {
        assert!(LangPath::is_valid_code("pt-br"));
    }

    #[test]
    fn path_traversal_is_rejected() {
        // Иначе через код языка можно уйти из каталога переводов.
        assert!(!LangPath::is_valid_code(".."));
        assert!(!LangPath::is_valid_code("../../etc/passwd"));
        assert!(!LangPath::is_valid_code("ru/../x"));
        assert!(!LangPath::is_valid_code(""));
        assert!(!LangPath::is_valid_code("-ru"));
        assert!(!LangPath::is_valid_code("ru-"));
        assert!(!LangPath::is_valid_code("ru/../.."));
    }

    #[test]
    fn file_is_named_after_the_language() {
        let p = LangPath::new("/home/u/.config/claxis/lang", "ru").unwrap();
        assert_eq!(p.file(), Path::new("/home/u/.config/claxis/lang/ru.toml"));
    }

    #[test]
    fn unknown_language_reports_error() {
        let err = LangPath::new("/tmp", "../x").unwrap_err();
        assert!(matches!(err, super::super::Error::UnknownLang { .. }));
    }
}

//! Алиасы: короткая строка, разворачивается в команду.
//!
//! Сравнивается напрямую: что напечатали, то и ищем. От раскладки зависит и
//! регистр важен: в qwerty срабатывает `dd`, в русской печатается `вв` и
//! алиас не сработает.

use std::fmt;
use std::str::FromStr;

use crate::error::Error;

/// Символы алиаса.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Alias(String);

impl Alias {
    /// Собрать алиас из символов. Пустой алиас — ошибка: его нельзя напечатать.
    pub fn new(symbols: &str) -> Result<Self, Error> {
        if symbols.is_empty()
            || !symbols
                .chars()
                .all(|c| !c.is_control() && !c.is_whitespace())
        {
            return Err(Error::BadAlias {
                got: symbols.to_string(),
            });
        }
        Ok(Self(symbols.to_string()))
    }

    /// Символы алиаса.
    pub fn symbols(&self) -> &str {
        &self.0
    }
}

impl FromStr for Alias {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        Self::new(text)
    }
}

impl fmt::Display for Alias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_is_compared_as_typed() {
        let alias = Alias::new("dd").unwrap();
        assert_eq!(alias.symbols(), "dd");
        assert_eq!(alias.to_string(), "dd");
    }

    #[test]
    fn case_is_kept_as_written() {
        // Регистр — часть алиаса, а не деталь отображения.
        assert_ne!(Alias::new("dd").unwrap(), Alias::new("dD").unwrap());
    }

    #[test]
    fn latin_and_cyrillic_aliases_are_different_ones() {
        // Отсюда и зависимость от раскладки: `вв` в qwerty не сработает.
        assert_ne!(Alias::new("вв").unwrap(), Alias::new("dd").unwrap());
    }

    #[test]
    fn an_empty_or_whitespace_alias_is_rejected() {
        for text in ["", " ", "\n", "d d", "\t"] {
            assert!(
                Alias::new(text).is_err(),
                "алиас {text:?} должен быть отвергнут"
            );
        }
    }
}

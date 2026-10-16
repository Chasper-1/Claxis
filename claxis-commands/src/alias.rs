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

/// Что напечатали: повтор и сам алиас.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Typed {
    /// Число повторов перед алиасом. `None` — повтор не задан.
    pub repeat: Option<u32>,
    /// Алиас без числа повторов.
    pub alias: Alias,
}

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

    /// Разобрать напечатанное: сначала повтор, потом алиас.
    ///
    /// Так же, как в командной строке: `5dd` — это пять строк, а не пять
    /// символов. Повтор отделён от алиаса, чтобы команда получила и то и
    /// другое и не гадала, где кончилось число.
    pub fn split(text: &str) -> Result<Typed, Error> {
        let digits = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        let (count, symbols) = text.split_at(digits);
        let repeat = if count.is_empty() {
            None
        } else {
            Some(count.parse().map_err(|_| Error::BadRepeatCount {
                got: count.to_string(),
            })?)
        };
        Ok(Typed {
            repeat,
            alias: Alias::new(symbols)?,
        })
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

    #[test]
    fn typed_count_comes_before_the_alias() {
        // `5dd` — это пять повторов алиаса `dd`, а не символы `5dd`.
        let typed = Alias::split("5dd").unwrap();
        assert_eq!(typed.repeat, Some(5));
        assert_eq!(typed.alias.symbols(), "dd");
    }

    #[test]
    fn typed_without_a_count_keeps_the_alias_whole() {
        let typed = Alias::split("dd").unwrap();
        assert_eq!(typed.repeat, None);
        assert_eq!(typed.alias.symbols(), "dd");
    }

    #[test]
    fn a_multi_digit_count_is_read_as_one_number() {
        let typed = Alias::split("100dd").unwrap();
        assert_eq!(typed.repeat, Some(100));
        assert_eq!(typed.alias.symbols(), "dd");
    }

    #[test]
    fn a_count_without_an_alias_is_an_error() {
        // `5` не команда: цифры без алиаса делать нечего.
        assert!(Alias::split("5").is_err());
        assert!(Alias::split("").is_err());
    }

    #[test]
    fn a_count_too_large_to_be_one_is_an_error_not_a_wraparound() {
        let error = Alias::split("99999999999999999999dd").unwrap_err();
        assert!(matches!(error, Error::BadRepeatCount { .. }));
    }
}

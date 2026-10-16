//! Уровни привязок: `global` → `state` → `regime`.
//!
//! Глобальный уровень — база, он блокирует остальные, потому что действует
//! везде. Проверка при нажатии идёт сверху вниз: сначала `global`, потом
//! локальный, и только потом выполняется команда.

use std::fmt;

use crate::error::Error;

/// Уровень, на котором стоит привязка.
///
/// Уровень записывается в конфиге путём секции: `global`, `edit`,
/// `edit.search`. `Regime` хранит оба куска, потому что режим без состояния
/// не имеет смысла: `search` существует только внутри `edit`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Level {
    /// Действует везде.
    Global,
    /// Действует в состоянии редактора.
    State(String),
    /// Действует в режиме состояния.
    Regime {
        /// Состояние, в котором живёт режим.
        state: String,
        /// Имя режима.
        regime: String,
    },
}

impl Level {
    /// Разобрать уровень из пути секции конфига.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bad = || Error::BadLevel {
            got: text.to_string(),
        };
        if text.is_empty() {
            return Err(bad());
        }
        let mut parts = text.split("::");
        let first = parts.next().ok_or_else(bad)?;
        let second = parts.next();
        if parts.next().is_some() {
            return Err(bad());
        }
        check_word(first).map_err(|_| bad())?;
        match second {
            None if first == "global" => Ok(Level::Global),
            None => Ok(Level::State(first.to_string())),
            Some(regime) => {
                check_word(regime).map_err(|_| bad())?;
                if first == "global" {
                    return Err(bad());
                }
                Ok(Level::Regime {
                    state: first.to_string(),
                    regime: regime.to_string(),
                })
            }
        }
    }

    /// Путь секции конфига для этого уровня.
    pub fn section(&self) -> String {
        match self {
            Level::Global => "global".to_string(),
            Level::State(state) => state.clone(),
            Level::Regime { state, regime } => format!("{state}::{regime}"),
        }
    }
}

fn check_word(word: &str) -> Result<(), ()> {
    if word.is_empty() || word.contains(':') || word.chars().any(|c| c.is_whitespace()) {
        return Err(());
    }
    Ok(())
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.section())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_levels_parse_from_their_section_paths() {
        assert_eq!(Level::parse("global").unwrap(), Level::Global);
        assert_eq!(
            Level::parse("edit").unwrap(),
            Level::State("edit".to_string())
        );
        assert_eq!(
            Level::parse("edit::search").unwrap(),
            Level::Regime {
                state: "edit".to_string(),
                regime: "search".to_string()
            }
        );
    }

    #[test]
    fn a_section_path_comes_back_the_same_way() {
        for text in ["global", "edit", "edit::search"] {
            assert_eq!(Level::parse(text).unwrap().section(), text);
        }
    }

    #[test]
    fn a_regime_without_a_state_is_rejected() {
        // `global` — не состояние: режим внутри него не имеет смысла.
        assert!(Level::parse("global::search").is_err());
    }

    #[test]
    fn nonsense_level_paths_are_rejected() {
        for text in ["", "::", "edit::search::deep", "edit::", " ed it"] {
            assert!(
                Level::parse(text).is_err(),
                "уровень {text:?} должен быть отвергнут"
            );
        }
    }

    #[test]
    fn levels_are_ordered_from_the_base_to_the_narrowest() {
        // Порядок задаёт, в каком порядке их проверяет резолвер.
        let mut levels = vec![
            Level::Regime {
                state: "edit".to_string(),
                regime: "search".to_string(),
            },
            Level::Global,
            Level::State("edit".to_string()),
        ];
        levels.sort();
        assert_eq!(
            levels,
            vec![
                Level::Global,
                Level::State("edit".to_string()),
                Level::Regime {
                    state: "edit".to_string(),
                    regime: "search".to_string()
                },
            ]
        );
    }
}

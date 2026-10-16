//! Имя команды: `:std::edit::move::up`.
//!
//! Иерархия жёсткая: без `:` не имя, после `:` — непустые куски без пустых.
//! Имя намеренно многословное: по нему читается смысл, и переводить его не
//! надо — это внутренняя логика, а не интерфейс.

use std::fmt;
use std::str::FromStr;

use crate::error::Error;

/// Разделитель кусков имени.
pub const SEPARATOR: &str = "::";

/// Имя команды — куски после `:`, без самого `:` и без разделителя.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CommandName {
    /// Куски имени, от пространства к уточнению.
    parts: Vec<String>,
}

impl CommandName {
    /// Собрать имя из кусков, как их виден человек: `["std", "edit"]`.
    pub fn new(parts: Vec<String>) -> Result<Self, Error> {
        if parts.is_empty() {
            return Err(Error::BadCommandName {
                got: String::new(),
                reason: "no parts".to_string(),
            });
        }
        for part in &parts {
            check_part(part)?;
        }
        Ok(Self { parts })
    }

    /// Разобрать имя из строки. `:` впереди обязателен.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let bad = |reason: &str| Error::BadCommandName {
            got: text.to_string(),
            reason: reason.to_string(),
        };
        let rest = text
            .strip_prefix(':')
            .ok_or_else(|| bad("no leading ':'"))?;
        if rest.is_empty() {
            return Err(bad("empty after ':'"));
        }
        let mut parts = Vec::new();
        for part in rest.split(SEPARATOR) {
            if part.is_empty() {
                return Err(bad("empty segment"));
            }
            check_part(part).map_err(|_| bad("bad characters in a segment"))?;
            parts.push(part.to_string());
        }
        Ok(Self { parts })
    }

    /// Куски имени.
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    /// Пространство имён: `std`, либо идентификатор плагина.
    pub fn namespace(&self) -> &str {
        &self.parts[0]
    }

    /// Есть ли у команды такое уточнение.
    pub fn has(&self, prefix: &[&str]) -> bool {
        prefix.len() <= self.parts.len()
            && self.parts[..prefix.len()]
                .iter()
                .zip(prefix)
                .all(|(part, want)| part == want)
    }

    /// Имя без ведущего `:` — так его ищут и так показывают.
    pub fn bare(&self) -> String {
        self.parts.join(SEPARATOR)
    }

    /// Продолжения имени по заданному префиксу — для автодополнения.
    ///
    /// Возвращает следующий кусок у каждой команды, у которой уже совпал
    /// префикс. Команда, совпавшая целиком, даёт пустой кусок: она готова.
    pub fn completions(&self, prefix: &[&str]) -> Option<&str> {
        if prefix.len() >= self.parts.len() {
            return None;
        }
        let matches = self.parts[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(part, want)| part == want);
        matches.then(|| self.parts[prefix.len()].as_str())
    }
}

fn check_part(part: &str) -> Result<(), Error> {
    let bad = || Error::BadCommandName {
        got: part.to_string(),
        reason: "bad characters in a segment".to_string(),
    };
    if part.chars().any(|c| c.is_whitespace() || c == ':') {
        return Err(bad());
    }
    Ok(())
}

impl FromStr for CommandName {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        Self::parse(text)
    }
}

impl fmt::Display for CommandName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(":")?;
        f.write_str(&self.bare())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_name_keeps_its_parts() {
        let name = CommandName::parse(":std::edit::move::up").unwrap();
        assert_eq!(name.namespace(), "std");
        assert_eq!(name.parts().len(), 4);
        assert_eq!(name.bare(), "std::edit::move::up");
        assert_eq!(name.to_string(), ":std::edit::move::up");
    }

    #[test]
    fn a_single_segment_name_is_allowed() {
        let name = CommandName::parse(":std").unwrap();
        assert_eq!(name.namespace(), "std");
    }

    #[test]
    fn a_name_without_the_colon_is_not_a_name() {
        assert!(matches!(
            CommandName::parse("std::edit"),
            Err(Error::BadCommandName { .. })
        ));
    }

    #[test]
    fn empty_segments_are_rejected() {
        for text in [":", ":std::", ":std::::edit", "::std"] {
            assert!(
                CommandName::parse(text).is_err(),
                "имя {text:?} должно быть отвергнуто"
            );
        }
    }

    #[test]
    fn whitespace_inside_a_name_is_rejected() {
        for text in [":std ::edit", ":std ed::it", ":std:\tedit"] {
            assert!(
                CommandName::parse(text).is_err(),
                "имя {text:?} должно быть отвергнуто"
            );
        }
    }

    #[test]
    fn a_plugin_namespace_parses_like_a_built_in_one() {
        let name = CommandName::parse(":my-plugin::file::save").unwrap();
        assert_eq!(name.namespace(), "my-plugin");
    }

    #[test]
    fn prefix_matching_is_by_segment_not_by_text() {
        let name = CommandName::parse(":std::edit::move::up").unwrap();
        assert!(name.has(&["std"]));
        assert!(name.has(&["std", "edit"]));
        assert!(!name.has(&["std", "file"]));
        // `ed` — это кусок, а не начало куска.
        assert!(!name.has(&["ed"]));
        assert!(!name.has(&["std", "edit", "move", "up", "extra"]));
    }

    #[test]
    fn a_completion_only_comes_after_a_matched_prefix() {
        let name = CommandName::parse(":std::edit::move::up").unwrap();
        assert_eq!(name.completions(&[]), Some("std"));
        assert_eq!(name.completions(&["std"]), Some("edit"));
        assert_eq!(name.completions(&["std", "edit"]), Some("move"));
        assert_eq!(name.completions(&["std", "edit", "move"]), Some("up"));
        // Дальше кусков нет: имя кончилось, дополнять нечего.
        assert_eq!(name.completions(&["std", "edit", "move", "up"]), None);
        // Чужой префикс не продолжается.
        assert_eq!(name.completions(&["file"]), None);
    }
}

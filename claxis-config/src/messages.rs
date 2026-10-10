//! Сообщения об ошибках и проблемах конфига.
//!
//! Конфиг не знает языка: он отдаёт структуру, а текст собирает каталог.

use claxis_text::substitute;

use crate::load::{Issue, Problem};

/// Каталог сообщений конфига.
///
/// Места под значения: `{path}`, `{reason}`, `{key}`, `{got}`, `{expected}`,
/// `{file}`, `{line}`, `{where}`.
pub trait Messages {
    /// Файл не прочитан.
    fn read_failed(&self, path: &str, reason: &str) -> String;
    /// Файл не записан.
    fn write_failed(&self, path: &str, reason: &str) -> String;
    /// Файл разобран неправильно.
    fn parse_failed(&self, path: &str, reason: &str) -> String;
    /// Ключа нет в схеме.
    fn unknown_key(&self, key: &str) -> String;
    /// Значение не того типа.
    fn bad_value(&self, key: &str, got: &str, expected: &str) -> String;
    /// Ключ задан не в своей секции.
    fn wrong_section(&self, key: &str, want: &str, got: &str) -> String;
    /// Запасного конфига нет.
    fn no_last_good(&self) -> String;
    /// Каталог конфига недоступен.
    fn config_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Сломано столько проблем, что работать не с чем.
    fn no_usable_config(&self, issues: usize) -> String;
    /// Ключ с таким именем попал в файл не в своей секции.
    fn unknown_key_in_file(&self, file: &str, key: &str) -> String;

    /// Текст проблемы конфига: `{where_}` — это `файл:строка`.
    fn issue(&self, issue: &Issue) -> String {
        let where_ = format!("{}:{}", issue.file, issue.line);
        match &issue.problem {
            Problem::UnknownKey => substitute(
                "{where}: unknown key {key}",
                &[("where", where_), ("key", issue.key.clone())],
            ),
            Problem::BadType { expected, got } => substitute(
                "{where}: key {key} has {got}, expected {expected}",
                &[
                    ("where", where_),
                    ("key", issue.key.clone()),
                    ("got", got.clone()),
                    ("expected", (*expected).to_string()),
                ],
            ),
            Problem::Unreadable(reason) => substitute(
                "{where}: cannot read file: {reason}",
                &[("where", where_), ("reason", reason.clone())],
            ),
        }
    }
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn read_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot read config {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn write_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot write config {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn parse_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot parse config {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn unknown_key(&self, key: &str) -> String {
        substitute("unknown key: {key}", &[("key", key.to_string())])
    }

    fn bad_value(&self, key: &str, got: &str, expected: &str) -> String {
        substitute(
            "bad value for {key}: {got}, expected {expected}",
            &[
                ("key", key.to_string()),
                ("got", got.to_string()),
                ("expected", expected.to_string()),
            ],
        )
    }

    fn wrong_section(&self, key: &str, want: &str, got: &str) -> String {
        substitute(
            "key {key} belongs to {want}, but is in {got}",
            &[
                ("key", key.to_string()),
                ("want", want.to_string()),
                ("got", got.to_string()),
            ],
        )
    }

    fn no_last_good(&self) -> String {
        "no saved valid config to fall back to".to_string()
    }

    fn config_dir_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot use config directory {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn no_usable_config(&self, issues: usize) -> String {
        substitute(
            "config has {issues} problems and no saved valid copy to fall back to",
            &[("issues", issues.to_string())],
        )
    }

    fn unknown_key_in_file(&self, file: &str, key: &str) -> String {
        substitute(
            "{file}: unknown key {key}",
            &[("file", file.to_string()), ("key", key.to_string())],
        )
    }
}

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    let unknown = Issue {
        file: "edit.toml",
        line: 2,
        key: "General.history_dept".to_string(),
        problem: Problem::UnknownKey,
    };
    let bad = Issue {
        file: "files.toml",
        line: 5,
        key: "General.snapshots_keep".to_string(),
        problem: Problem::BadType {
            expected: "a whole number",
            got: "\"много\"".to_string(),
        },
    };
    let unreadable = Issue {
        file: "config.toml",
        line: 0,
        key: String::new(),
        problem: Problem::Unreadable("permission denied".to_string()),
    };
    vec![
        ("unknown_key", m.issue(&unknown)),
        ("bad_value", m.issue(&bad)),
        ("unreadable", m.issue(&unreadable)),
        (
            "read_failed",
            m.read_failed("/home/u/.config/claxis/edit.toml", "no such file"),
        ),
        (
            "write_failed",
            m.write_failed("/home/u/.config/claxis/edit.toml", "read-only"),
        ),
        (
            "parse_failed",
            m.parse_failed("/home/u/.config/claxis/edit.toml", "line 4: unclosed quote"),
        ),
        ("unknown_key", m.unknown_key("General.history_dept")),
        (
            "bad_value",
            m.bad_value("General.snapshots_keep", "\"много\"", "a whole number"),
        ),
        (
            "wrong_section",
            m.wrong_section("history_depth", "edit.toml", "files.toml"),
        ),
        ("no_last_good", m.no_last_good()),
        (
            "config_dir_failed",
            m.config_dir_failed("/home/u/.config/claxis", "permission denied"),
        ),
        ("no_usable_config", m.no_usable_config(4)),
        (
            "unknown_key_in_file",
            m.unknown_key_in_file("files.toml", "General.history_depth"),
        ),
    ]
}

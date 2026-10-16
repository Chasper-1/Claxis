//! Сообщения об ошибках системы команд.
//!
//! Крейт не знает языка: отдаёт ошибку со структурой, текст собирает каталог.
//! По умолчанию — английский. Имена в фигурных скобках — места под значения.

use crate::error::Error;

/// Каталог сообщений системы команд.
pub trait Messages {
    /// Команда не зарегистрирована: `{name}`.
    fn unknown_command(&self, name: &str) -> String;

    /// Имя команды не разобрано: `{got}`, `{reason}`.
    fn bad_command_name(&self, got: &str, reason: &str) -> String;

    /// Число повторов не разобрано: `{got}`.
    fn bad_repeat_count(&self, got: &str) -> String;

    /// Одна и та же комбинация на двух уровнях: `{key}`, `{first}`,
    /// `{second}`.
    fn binding_conflict(&self, key: &str, first: &str, second: &str) -> String;

    /// Уровень привязки не разобран: `{got}`.
    fn bad_level(&self, got: &str) -> String;

    /// Символ алиаса разобран неверно: `{got}`.
    fn bad_alias(&self, got: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn unknown_command(&self, name: &str) -> String {
        substitute("unknown command: {name}", &[("name", name.to_string())])
    }

    fn bad_command_name(&self, got: &str, reason: &str) -> String {
        substitute(
            "malformed command name \"{got}\": {reason}",
            &[("got", got.to_string()), ("reason", reason.to_string())],
        )
    }

    fn bad_repeat_count(&self, got: &str) -> String {
        substitute(
            "repeat count is not a number: {got}",
            &[("got", got.to_string())],
        )
    }

    fn binding_conflict(&self, key: &str, first: &str, second: &str) -> String {
        substitute(
            "\"{key}\" is bound twice: {first} and {second}",
            &[
                ("key", key.to_string()),
                ("first", first.to_string()),
                ("second", second.to_string()),
            ],
        )
    }

    fn bad_level(&self, got: &str) -> String {
        substitute(
            "malformed level \"{got}\": expected \"global\" or \"state::regime\"",
            &[("got", got.to_string())],
        )
    }

    fn bad_alias(&self, got: &str) -> String {
        substitute(
            "malformed alias \"{got}\": expected printable characters",
            &[("got", got.to_string())],
        )
    }
}

pub use claxis_text::substitute;

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        ("unknown_command", m.unknown_command(":std::edit::move::up")),
        (
            "bad_command_name",
            m.bad_command_name(":std::::edit", "empty segment"),
        ),
        ("bad_repeat_count", m.bad_repeat_count("many")),
        (
            "binding_conflict",
            m.binding_conflict("dd", "global", "edit"),
        ),
        ("bad_level", m.bad_level("edit..search")),
        ("bad_alias", m.bad_alias("a\nb")),
    ]
}

/// Текст ошибки на языке по умолчанию.
pub fn text(error: &Error) -> String {
    match error {
        Error::UnknownCommand { name } => En.unknown_command(name),
        Error::BadCommandName { got, reason } => En.bad_command_name(got, reason),
        Error::BadRepeatCount { got } => En.bad_repeat_count(got),
        Error::BindingConflict { key, first, second } => En.binding_conflict(key, first, second),
        Error::BadLevel { got } => En.bad_level(got),
        Error::BadAlias { got } => En.bad_alias(got),
    }
}

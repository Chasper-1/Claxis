//! Что пошло не так в системе команд.

use std::fmt;

use crate::messages::{Messages, text};

/// Ошибка системы команд.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// Команда не зарегистрирована: `{name}`.
    UnknownCommand {
        /// Имя команды, как его искали.
        name: String,
    },
    /// Имя команды не разобрано: `{got}`, `{reason}`.
    BadCommandName {
        /// Что пришло.
        got: String,
        /// Почему не подошло.
        reason: String,
    },
    /// Число повторов не разобрано: `{got}`.
    BadRepeatCount {
        /// Что пришло на месте числа.
        got: String,
    },
    /// Одна и та же комбинация на двух уровнях: `{key}`, `{first}`, `{second}`.
    BindingConflict {
        /// Что нажали или напечатали.
        key: String,
        /// Уровень, который занял первым.
        first: String,
        /// Уровень, который столкнулся с ним.
        second: String,
    },
    /// Уровень привязки не разобран: `{got}`.
    BadLevel {
        /// Что пришло.
        got: String,
    },
    /// Символ алиаса разобран неверно: `{got}`.
    BadAlias {
        /// Что пришло.
        got: String,
    },
}

impl Error {
    /// Текст ошибки на языке каталога.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::UnknownCommand { name } => messages.unknown_command(name),
            Error::BadCommandName { got, reason } => messages.bad_command_name(got, reason),
            Error::BadRepeatCount { got } => messages.bad_repeat_count(got),
            Error::BindingConflict { key, first, second } => {
                messages.binding_conflict(key, first, second)
            }
            Error::BadLevel { got } => messages.bad_level(got),
            Error::BadAlias { got } => messages.bad_alias(got),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        text(self)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for Error {}

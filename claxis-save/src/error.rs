//! Что пошло не так при проверке и сохранении.

use std::fmt;

use crate::messages::Messages;

/// Ошибка сохранения.
#[derive(Debug)]
pub enum Error {
    /// Диск отказал: его ошибка уже с путём и причиной.
    Disk(claxis_disk::Error),
    /// Срок годности не разобран: `{value}`.
    Retention {
        /// Что не разобралось.
        value: String,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::Disk(e) => messages.disk_error(e),
            Error::Retention { value } => messages.retention_failed(value),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for Error {}

/// Результат проверки и сохранения.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_message() {
        // Ошибка без текста — это ошибка, которую человек не увидит: молчание
        // хуже, чем неудобное сообщение.
        let errors = [
            Error::Disk(claxis_disk::Error::Query {
                reason: "locked".to_string(),
            }),
            Error::Retention {
                value: "2w".to_string(),
            },
        ];
        for error in errors {
            let text = error.text();
            assert!(!text.is_empty(), "{error:?} осталась без текста");
            assert_eq!(format!("{error}"), text);
        }
    }
}

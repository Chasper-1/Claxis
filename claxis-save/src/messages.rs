//! Сообщения о сохранении.
//!
//! Крейт не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use claxis_text::substitute;

/// Каталог сообщений о сохранении.
///
/// Места под значения: `{path}`, `{value}`.
pub trait Messages {
    /// Диск отказал: делегирует в каталог диска.
    fn disk_error(&self, error: &claxis_disk::Error) -> String;
    /// Записывать файл отказались: `{path}` и чем.
    ///
    /// Отказ — не ошибка диска, а решение политики: файл на диске меняли
    /// снаружи, и затирать его отданной работой значило бы потерять чужое.
    fn save_refused(&self, path: &str, refusal: &crate::Refusal) -> String;
    /// Срок годности не разобран: `{value}`.
    fn retention_failed(&self, value: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn disk_error(&self, error: &claxis_disk::Error) -> String {
        error.message(&claxis_disk::messages::En)
    }

    fn save_refused(&self, path: &str, refusal: &crate::Refusal) -> String {
        match refusal {
            crate::Refusal::Changed => substitute(
                "file {path} was changed outside the editor; nothing was written,\n compare the two by hand",
                &[("path", path.to_string())],
            ),
            crate::Refusal::Unmarked => substitute(
                "there is no freshness mark of {path}; the file could have been changed",
                &[("path", path.to_string())],
            ),
            crate::Refusal::Gone => substitute(
                "file {path} does not exist; nothing was written",
                &[("path", path.to_string())],
            ),
        }
    }

    fn retention_failed(&self, value: &str) -> String {
        substitute(
            "cannot read {value} as a lifetime: the form is a number with d, h, m or s,\n or a bare number of seconds",
            &[("value", value.to_string())],
        )
    }
}

/// Все сообщения крейта с примерами значений.
///
/// Без этого переводчик не видит, что вообще бывает: каталог строится по
/// примеру, а ключ, которого в примере нет, никогда не переведётся.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        (
            "disk_error",
            m.disk_error(&claxis_disk::Error::Query {
                reason: "database is locked".to_string(),
            }),
        ),
        (
            "save_refused",
            m.save_refused("/home/u/project/src/main.rs", &crate::Refusal::Changed),
        ),
        ("retention_failed", m.retention_failed("2w")),
    ]
}

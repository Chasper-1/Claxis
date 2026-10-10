//! Сообщения об ошибках сессии.
//!
//! Сессия не знает языка: она отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use crate::api::text::{group_digits, substitute};
/// Каталог сообщений об ошибках сессии.
///
/// Места под значения: `{path}`, `{reason}`, `{bytes}`, `{limit_gb}`.
pub trait Messages {
    /// Хранилище не открылось.
    fn store_failed(&self, path: &str, reason: &str) -> String;
    /// Файл документа не прочитан.
    fn read_file_failed(&self, path: &str, reason: &str) -> String;
    /// Ошибка буфера: делегирует в каталог буфера.
    fn buffer_error(&self, error: &crate::api::buffer::Error) -> String;
    /// Каталог конфига недоступен: `{path}`, `{reason}`.
    fn config_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Сломано столько проблем, что работать не с чем: `{issues}`.
    fn no_usable_config(&self, issues: usize) -> String;
    /// Файл больше, чем помещается в `u32`.
    ///
    /// Байты — точное число, гигабайты приложены для удобства. Считаются в
    /// десятичных, 10⁹, как обычно.
    ///
    /// `{bytes}` и `{limit_bytes}` — точные размеры, `{size_gb}` и
    /// `{limit_gb}` — те же числа в гигабайтах.
    fn file_too_large(&self, bytes: u64, size_gb: f64, limit_bytes: u64, limit_gb: f64) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn store_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot open store at {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn read_file_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot read file {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn buffer_error(&self, error: &crate::api::buffer::Error) -> String {
        error.message(&crate::api::buffer::En)
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

    fn file_too_large(&self, bytes: u64, size_gb: f64, limit_bytes: u64, limit_gb: f64) -> String {
        substitute(
            "file is too large: {bytes} bytes ({size_gb} GB),\n\
             the maximum is {limit_bytes} bytes ({limit_gb} GB)",
            &[
                ("bytes", group_digits(bytes)),
                ("size_gb", format!("{size_gb:.1}")),
                ("limit_bytes", group_digits(limit_bytes)),
                ("limit_gb", format!("{limit_gb:.1}")),
            ],
        )
    }
}

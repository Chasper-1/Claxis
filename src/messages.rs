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
    fn buffer_error(&self, error: &crate::api::rope::Error) -> String;
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

    fn buffer_error(&self, error: &crate::api::rope::Error) -> String {
        error.message(&crate::api::rope::En)
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

/// Все сообщения сессии с примерами значений.
///
/// Без этого переводчик не видит, что вообще бывает: каталог строится по
/// примеру, а ключ, которого в примере нет, никогда не переведётся.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        (
            "store_failed",
            m.store_failed("/home/user/.cache/claxis", "no access"),
        ),
        (
            "read_file_failed",
            m.read_file_failed("/home/user/notes.txt", "no such file"),
        ),
        (
            "buffer_error",
            m.buffer_error(&crate::api::rope::Error::TruncatedHistory),
        ),
        (
            "config_dir_failed",
            m.config_dir_failed("/home/user/.config/claxis", "no access"),
        ),
        ("no_usable_config", m.no_usable_config(2)),
        (
            "file_too_large",
            m.file_too_large(5_000_000_000, 5.0, u32::MAX as u64, 4.2),
        ),
    ]
}

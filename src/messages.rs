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
    /// Ошибка команды: делегирует в каталог команд.
    fn commands_error(&self, error: &crate::api::commands::Error) -> String;
    /// Каталог конфига недоступен: `{path}`, `{reason}`.
    fn config_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Домашнего каталога нет: конфиг положить некуда.
    fn no_home_dir(&self) -> String;
    /// Снапшот не записался: `{path}` — какой файл, `{reason}` — почему.
    ///
    /// Отдельно и громко: без этого сообщения человек закрывает редактор и
    /// теряет работу, думая, что она сохранена.
    fn snapshot_failed(&self, path: &str, reason: &str) -> String;

    /// За папкой следить не вышло: `{where}` — где и почему. Правки применятся
    /// при перезапуске.
    fn not_watched(&self, place: &str) -> String;

    /// Наблюдение за файлами сломалось: `{reason}`. Правки перестанут
    /// применяться сами, но ничто не пропадёт.
    fn watch_failed(&self, reason: &str) -> String;

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

    fn commands_error(&self, error: &crate::api::commands::Error) -> String {
        error.message(&crate::api::commands::En)
    }

    fn config_dir_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot use config directory {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn no_home_dir(&self) -> String {
        "cannot find the home directory, so there is nowhere to keep the config".to_string()
    }

    fn snapshot_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot save a snapshot of {path}: {reason}; your work will not survive a restart",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn not_watched(&self, place: &str) -> String {
        substitute(
            "cannot watch {place}; changes there will apply on restart",
            &[("place", place.to_string())],
        )
    }

    fn watch_failed(&self, reason: &str) -> String {
        substitute(
            "the file watcher failed: {reason}; config changes will no longer apply by themselves",
            &[("reason", reason.to_string())],
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
            m.buffer_error(&crate::api::buffer::Error::TruncatedHistory),
        ),
        (
            "commands_error",
            m.commands_error(&crate::api::commands::Error::UnknownCommand {
                name: ":std::edit::undo".to_string(),
            }),
        ),
        (
            "config_dir_failed",
            m.config_dir_failed("/home/user/.config/claxis", "no access"),
        ),
        ("no_home_dir", m.no_home_dir()),
        (
            "snapshot_failed",
            m.snapshot_failed("/home/user/notes.txt", "disk is full"),
        ),
        ("no_usable_config", m.no_usable_config(2)),
        (
            "not_watched",
            m.not_watched("/home/user/.config/claxis/theme: too many open files"),
        ),
        (
            "file_too_large",
            m.file_too_large(5_000_000_000, 5.0, u32::MAX as u64, 4.2),
        ),
        (
            "watch_failed",
            m.watch_failed("the notification queue overflowed"),
        ),
    ]
}

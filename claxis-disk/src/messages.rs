//! Сообщения об ошибках диска.
//!
//! Крейт не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use claxis_text::{group_digits, substitute};

/// Каталог сообщений об ошибках диска.
///
/// Места под значения: `{path}`, `{reason}`, `{file}`.
pub trait Messages {
    /// Файл не записан.
    fn write_failed(&self, path: &str, reason: &str) -> String;
    /// Файл не прочитан.
    fn read_failed(&self, path: &str, reason: &str) -> String;
    /// Каталог не создан.
    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Файл не переименован после записи.
    fn rename_failed(&self, path: &str, reason: &str) -> String;
    /// Подписка на каталог не вышла.
    fn watch_failed(&self, path: &str, reason: &str) -> String;
    /// Система наблюдения не ответила.
    fn watch_system_failed(&self, reason: &str) -> String;
    /// Файл базы не открылся.
    fn open_failed(&self, path: &str, reason: &str) -> String;
    /// Запрос к базе не выполнился.
    fn query_failed(&self, reason: &str) -> String;
    /// Снапшота для файла нет.
    fn no_snapshot(&self, file: &str) -> String;

    /// Снапшот испорчен извне: `{file}`, `{seq}`, `{expected}`, `{got}`.
    fn corrupted(&self, file: &str, seq: u64, expected: u64, got: u64) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn write_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot write {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn read_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot read {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot create directory {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn rename_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot put the written file in place of {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn watch_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot watch {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn watch_system_failed(&self, reason: &str) -> String {
        substitute(
            "the file watcher failed: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn open_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot open store at {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn query_failed(&self, reason: &str) -> String {
        substitute(
            "store query failed: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn no_snapshot(&self, file: &str) -> String {
        substitute(
            "no snapshot stored for {file}",
            &[("file", file.to_string())],
        )
    }

    fn corrupted(&self, file: &str, seq: u64, expected: u64, got: u64) -> String {
        substitute(
            "snapshot {seq} of {file} was changed outside the editor: checksum is {got}, expected {expected}",
            &[
                ("file", file.to_string()),
                ("seq", group_digits(seq)),
                ("expected", group_digits(expected)),
                ("got", group_digits(got)),
            ],
        )
    }
}

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        (
            "write_failed",
            m.write_failed("/home/u/project/config.toml", "no space left on device"),
        ),
        (
            "read_failed",
            m.read_failed("/home/u/project/config.toml", "permission denied"),
        ),
        (
            "prepare_dir_failed",
            m.prepare_dir_failed("/home/u/.cache/claxis", "read-only file system"),
        ),
        (
            "rename_failed",
            m.rename_failed("/home/u/project/config.toml", "device is busy"),
        ),
        (
            "watch_failed",
            m.watch_failed("/home/u/.config/claxis", "too many open files"),
        ),
        (
            "watch_system_failed",
            m.watch_system_failed("the notification queue overflowed"),
        ),
        (
            "open_failed",
            m.open_failed("/home/u/.cache/claxis/store.db", "disk is read-only"),
        ),
        ("query_failed", m.query_failed("database is locked")),
        ("no_snapshot", m.no_snapshot("/home/u/project/src/main.rs")),
        (
            "corrupted",
            m.corrupted(
                "/home/u/project/src/main.rs",
                1_024,
                0x1a2b_3c4d_5e6f_7788,
                0,
            ),
        ),
    ]
}

//! Сообщения об ошибках хранилища.
//!
//! Хранилище не знает языка: оно отдаёт ошибку со структурными данными, а
//! текст собирает каталог. По умолчанию — английский.

use claxis_text::substitute;

/// Каталог сообщений об ошибках хранилища.
///
/// Места под значения: `{path}`, `{reason}`, `{file}`.
pub trait Messages {
    /// Файл базы не открылся.
    fn open_failed(&self, path: &str, reason: &str) -> String;
    /// Каталог для базы не создался.
    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Запрос к базе не выполнился.
    fn query_failed(&self, reason: &str) -> String;
    /// Снапшота для файла нет.
    fn no_snapshot(&self, file: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn open_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot open store at {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot create store directory {path}: {reason}",
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
}
/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        (
            "open_failed",
            m.open_failed("/home/u/.cache/claxis/store.db", "disk is read-only"),
        ),
        (
            "prepare_dir_failed",
            m.prepare_dir_failed("/home/u/.cache/claxis", "no space left on device"),
        ),
        ("query_failed", m.query_failed("database is locked")),
        ("no_snapshot", m.no_snapshot("/home/u/project/src/main.rs")),
    ]
}

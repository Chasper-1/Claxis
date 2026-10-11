//! Сообщения об ошибках темы.
//!
//! Тема не знает языка: она отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use claxis_text::substitute;

/// Каталог сообщений об ошибках темы.
///
/// Места под значения: `{name}`, `{path}`, `{dir}`, `{available}`.
pub trait Messages {
    /// Темы с таким именем нет.
    fn unknown_theme(&self, name: &str, available: &[String]) -> String;
    /// Папка с темами не прочитана.
    fn read_dir_failed(&self, dir: &str, reason: &str) -> String;
    /// Имя темы не годится: путь уводит за пределы папки.
    fn invalid_theme_name(&self, name: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn unknown_theme(&self, name: &str, available: &[String]) -> String {
        substitute(
            "unknown theme: {name}. Available themes: {available}",
            &[
                ("name", name.to_string()),
                ("available", available.join(", ")),
            ],
        )
    }

    fn read_dir_failed(&self, dir: &str, reason: &str) -> String {
        substitute(
            "cannot read theme folder {dir}: {reason}",
            &[("dir", dir.to_string()), ("reason", reason.to_string())],
        )
    }

    fn invalid_theme_name(&self, name: &str) -> String {
        substitute("invalid theme name: {name}", &[("name", name.to_string())])
    }
}

/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        (
            "unknown_theme",
            m.unknown_theme(
                "everforest",
                &["default".to_string(), "everforest".to_string()],
            ),
        ),
        (
            "read_dir_failed",
            m.read_dir_failed("/home/u/.config/claxis/theme", "permission denied"),
        ),
        (
            "invalid_theme_name",
            m.invalid_theme_name("../../etc/passwd"),
        ),
    ]
}

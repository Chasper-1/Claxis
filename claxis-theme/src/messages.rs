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
    /// Строка не похожа на код цвета.
    fn not_a_colour(&self, value: &str) -> String;
    /// Канал или альфа цвета вне допустимого диапазона.
    fn colour_channel_out_of_range(&self, value: &str) -> String;
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

    fn not_a_colour(&self, value: &str) -> String {
        substitute("not a colour: {value}", &[("value", value.to_string())])
    }

    fn colour_channel_out_of_range(&self, value: &str) -> String {
        substitute(
            "colour channel out of range: {value}",
            &[("value", value.to_string())],
        )
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
        ("not_a_colour", m.not_a_colour("#xyz")),
        (
            "colour_channel_out_of_range",
            m.colour_channel_out_of_range("rgb(300, 0, 0)"),
        ),
    ]
}

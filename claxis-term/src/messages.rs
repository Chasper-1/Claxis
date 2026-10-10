//! Сообщения об ошибках терминала.
//!
//! Терминал не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

/// Каталог сообщений об ошибках терминала.
pub trait Messages {
    /// Терминал не поддерживает kitty keyboard protocol.
    fn no_kitty_protocol(&self) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn no_kitty_protocol(&self) -> String {
        "terminal does not support kitty keyboard protocol. \
         A terminal with support is required: kitty, WezTerm, foot, Ghostty, \
         Windows Terminal configuration. The editor cannot work without the protocol."
            .to_string()
    }
}
/// Все сообщения крейте с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![("no_kitty_protocol", m.no_kitty_protocol())]
}

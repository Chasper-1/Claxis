//! Что пошло не так в терминале.

use std::fmt;

use crate::messages::Messages;

/// Ошибка терминала.
#[derive(Debug)]
pub enum TermError {
    /// Терминал не поддерживает kitty keyboard protocol.
    ///
    /// Без протокола нельзя отличить отпускание от нового нажатия, а модель
    /// ввода требует событий отпускания. Поэтому это отказ, а не деградация.
    NoKittyProtocol,
}

impl TermError {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            TermError::NoKittyProtocol => messages.no_kitty_protocol(),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for TermError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for TermError {}

//! Сообщения бинарного крейта.
//!
//! Ничего не зашито: текст собирается каталогом, английский по умолчанию.

/// Каталог сообщений бинарника.
pub trait Messages {
    /// Терминал не открылся: `{reason}` — почему.
    fn terminal_failed(&self, reason: &str) -> String;
    /// Сессия не открылась: `{reason}` — почему.
    fn session_failed(&self, reason: &str) -> String;
    /// Событие терминала показано как есть: `{event}` — его запись.
    fn event_line(&self, event: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn terminal_failed(&self, reason: &str) -> String {
        claxis_text::substitute(
            "cannot open terminal: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn session_failed(&self, reason: &str) -> String {
        claxis_text::substitute(
            "cannot open session: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn event_line(&self, event: &str) -> String {
        claxis_text::substitute("{event}", &[("event", event.to_string())])
    }
}

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
    /// Конфиг сломан, редактор пошёл на запасной.
    fn using_last_good(&self) -> String;
    /// Строка usage: `{usage}` — как вызывается.
    fn usage(&self, usage: &str) -> String;
    /// Без команды открывается редактор.
    fn opens_editor(&self) -> String;
    /// Заголовок списка команд.
    fn commands_heading(&self) -> String;
    /// Строка списка команд: `{name}` и `{what}`.
    fn command_line(&self, name: &str, what: &str) -> String;
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn terminal_failed(&self, reason: &str) -> String {
        crate::api::text::substitute(
            "cannot open terminal: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn session_failed(&self, reason: &str) -> String {
        crate::api::text::substitute(
            "cannot open session: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn event_line(&self, event: &str) -> String {
        crate::api::text::substitute("{event}", &[("event", event.to_string())])
    }

    fn using_last_good(&self) -> String {
        "config is broken, using the last saved good copy".to_string()
    }

    fn usage(&self, usage: &str) -> String {
        crate::api::text::substitute(usage, &[("usage", usage.to_string())])
    }

    fn opens_editor(&self) -> String {
        "Without a command the editor opens.".to_string()
    }

    fn commands_heading(&self) -> String {
        "Commands:".to_string()
    }

    fn command_line(&self, name: &str, what: &str) -> String {
        crate::api::text::substitute(
            "  {name}   {what}",
            &[("name", name.to_string()), ("what", what.to_string())],
        )
    }
}

/// Все сообщения бинарника с примерами значений.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        ("terminal_failed", m.terminal_failed("terminal closed")),
        ("session_failed", m.session_failed("cannot open store")),
        ("event_line", m.event_line("KeyPressed { key: Char('a') }")),
    ]
}

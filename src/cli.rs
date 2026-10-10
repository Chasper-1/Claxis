//! Команды командной строки.
//!
//! Без них редактор запускается просто: `claxis`. С ними появляются
//! отладочные и служебные команды, которыми пользуются не при работе с
//! текстом, а при настройке и разборе проблем.

use std::io::Write;

use crate::api::buffer;
use crate::api::config;
use crate::api::i18n;
use crate::api::store;
use crate::api::term;
use crate::main_messages;

/// Что просил пользователь в командной строке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Запустить редактор.
    Run,
    /// Показать все сообщения всех крейтов.
    Messages,
    /// Показать справку.
    Help,
}

/// Разобрать аргументы командной строки.
///
/// Аргументы, которых мы не знаем, — не ошибка: они могут принадлежать
/// редактору, путь к файлу выглядит как аргумент.
pub fn parse(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        Some("messages") => Command::Messages,
        Some("help") | Some("--help") | Some("-h") => Command::Help,
        _ => Command::Run,
    }
}

/// Вывести все сообщения всех крейтов.
///
/// Нужны двум: починить текст и показать переводчику, что вообще есть.
/// Каждое сообщение выводится с настоящими значениями вместо имён в
/// фигурных скобках, иначе переводчик не поймёт, что подставляется.
pub fn print_messages(out: &mut impl Write) -> std::io::Result<()> {
    let crates: [(&str, Vec<(&str, String)>); 6] = [
        ("claxis-buffer", buffer::messages::samples()),
        ("claxis-store", store::messages::samples()),
        ("claxis-term", term::messages::samples()),
        ("claxis-i18n", i18n::messages::samples()),
        ("claxis-config", config::messages::samples()),
        ("Claxis", main_messages::samples()),
    ];

    for (name, messages) in crates {
        writeln!(out, "{name}")?;
        for (key, text) in messages {
            writeln!(out, "  {key}: {text}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// Справка по командной строке.
///
/// Текст идёт через каталог: справку читает пользователь, значит она такая же
/// переводимая, как всё остальное.
pub fn print_help(out: &mut impl Write, m: &dyn main_messages::Messages) -> std::io::Result<()> {
    writeln!(out, "{}", m.usage("claxis [command] [file]"))?;
    writeln!(out)?;
    writeln!(out, "{}", m.opens_editor())?;
    writeln!(out)?;
    writeln!(out, "{}", m.commands_heading())?;
    for (name, what) in COMMANDS {
        writeln!(out, "{}", m.command_line(name, what))?;
    }
    Ok(())
}

/// Команды и что они делают. Значения переводятся каталогом.
const COMMANDS: [(&str, &str); 2] = [
    ("messages", "show every message of every crate"),
    ("help", "this help"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{i18n, store, term};

    fn out_of(command: Command) -> String {
        use crate::main_messages::En;
        let mut buf: Vec<u8> = Vec::new();
        match command {
            Command::Messages => print_messages(&mut buf).unwrap(),
            Command::Help => print_help(&mut buf, &En).unwrap(),
            Command::Run => {}
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn arguments_are_parsed() {
        assert_eq!(parse(&[]), Command::Run);
        assert_eq!(parse(&["file.rs".to_string()]), Command::Run);
        assert_eq!(parse(&["messages".to_string()]), Command::Messages);
        assert_eq!(parse(&["--help".to_string()]), Command::Help);
    }

    #[test]
    fn every_crate_appears_in_the_message_list() {
        // Если крейт забыли, его сообщения нечем будет починить.
        let text = out_of(Command::Messages);
        for crate_name in [
            "claxis-buffer",
            "claxis-store",
            "claxis-term",
            "claxis-i18n",
            "claxis-config",
            "Claxis",
        ] {
            assert!(text.contains(crate_name), "{crate_name} не попал в вывод");
        }
    }

    #[test]
    fn messages_are_shown_with_real_values() {
        // Переводчику нужен настоящий пример, а не имя в фигурных скобках.
        let text = out_of(Command::Messages);
        assert!(text.contains("range 1 000..2 000"), "{text}");
        assert!(text.contains("edit.toml:2:"), "нет файла и строки: {text}");
        // Незаполненных имён быть не должно.
        assert!(!text.contains("{capacity}"), "осталось имя без значения");
        assert!(!text.contains("{doc_len}"), "осталось имя без значения");
    }

    #[test]
    fn every_crate_has_samples() {
        // Пустой список — значит сообщения забыли перечислить.
        assert!(!buffer::messages::samples().is_empty());
        assert!(!store::messages::samples().is_empty());
        assert!(!term::messages::samples().is_empty());
        assert!(!i18n::messages::samples().is_empty());
        assert!(!config::messages::samples().is_empty());
        assert!(!main_messages::samples().is_empty());
    }

    #[test]
    fn help_lists_every_command() {
        let text = out_of(Command::Help);
        assert!(text.contains("messages"), "{text}");
        assert!(text.contains("help"), "{text}");
    }

    #[test]
    fn help_comes_from_the_catalog() {
        // Справка читает пользователь, значит она переводима наравне со всем
        // остальным. Если появится перевод, он обязан сюда попасть.
        use crate::main_messages::En;
        struct Ru;
        impl main_messages::Messages for Ru {
            fn terminal_failed(&self, r: &str) -> String {
                format!("нет терминала: {r}")
            }
            fn session_failed(&self, r: &str) -> String {
                format!("нет сессии: {r}")
            }
            fn event_line(&self, e: &str) -> String {
                format!("событие: {e}")
            }
            fn using_last_good(&self) -> String {
                "взят запасной".to_string()
            }
            fn usage(&self, u: &str) -> String {
                format!("запуск: {u}")
            }
            fn opens_editor(&self) -> String {
                "откроется редактор".to_string()
            }
            fn commands_heading(&self) -> String {
                "Команды:".to_string()
            }
            fn command_line(&self, n: &str, w: &str) -> String {
                format!("  {n} — {w}")
            }
        }
        let _ = En;
        let mut buf: Vec<u8> = Vec::new();
        print_help(&mut buf, &Ru).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(
            text.contains("откроется редактор"),
            "перевод не попал: {text}"
        );
        assert!(text.contains("Команды:"), "{text}");
    }
}

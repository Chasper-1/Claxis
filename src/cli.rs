//! Команды командной строки.
//!
//! Без них редактор запускается просто: `claxis`. С ними появляются
//! отладочные и служебные команды, которыми пользуются не при работе с
//! текстом, а при настройке и разборе проблем.

use std::io::Write;

use crate::api::buffer;
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
        ("claxis-i18n", i18n::samples()),
        ("claxis-config", config::samples()),
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
pub fn print_help(out: &mut impl Write) -> std::io::Result<()> {
    writeln!(out, "claxis [команда] [файл]")?;
    writeln!(out)?;
    writeln!(out, "Без команды открывается редактор.")?;
    writeln!(out)?;
    writeln!(out, "Команды:")?;
    writeln!(out, "  messages   показать все сообщения всех крейтов")?;
    writeln!(out, "  help       эта справка")?;
    Ok(())
}

/// Сообщения крейта конфига, собранные здесь, чтобы команда не знала про него.
mod config {
    /// Все сообщения конфига с примерами значений.
    pub fn samples() -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let issue = claxis_config::load::Issue {
            file: "edit.toml",
            line: 2,
            key: "General.history_dept".to_string(),
            problem: claxis_config::Problem::UnknownKey,
        };
        out.push(("unknown_key", issue.text()));
        let bad = claxis_config::load::Issue {
            file: "files.toml",
            line: 5,
            key: "General.snapshots_keep".to_string(),
            problem: claxis_config::Problem::BadType {
                expected: "a whole number",
                got: String::from("\"много\""),
            },
        };
        out.push(("bad_value", bad.text()));
        let unreadable = claxis_config::load::Issue {
            file: "config.toml",
            line: 0,
            key: String::new(),
            problem: claxis_config::Problem::Unreadable("permission denied".to_string()),
        };
        out.push(("unreadable", unreadable.text()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out_of(command: Command) -> String {
        let mut buf: Vec<u8> = Vec::new();
        match command {
            Command::Messages => print_messages(&mut buf).unwrap(),
            Command::Help => print_help(&mut buf).unwrap(),
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
        assert!(!i18n::samples().is_empty());
        assert!(!config::samples().is_empty());
        assert!(!main_messages::samples().is_empty());
    }

    #[test]
    fn help_lists_every_command() {
        let text = out_of(Command::Help);
        assert!(text.contains("messages"), "{text}");
        assert!(text.contains("help"), "{text}");
    }
}

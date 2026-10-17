//! Команды командной строки.
//!
//! Без них редактор запускается просто: `claxis`. С ними появляются
//! отладочные и служебные команды, которыми пользуются не при работе с
//! текстом, а при настройке и разборе проблем.

use std::io::Write;

use crate::api::buffer;
use crate::api::config;
use crate::api::disk;
use crate::api::i18n;
use crate::api::term;
use crate::main_messages;

/// Что просил пользователь в командной строке.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Запустить редактор, открыв файл, если он указан.
    Run {
        /// Путь к файлу: `None` — редактор без файла.
        path: Option<std::path::PathBuf>,
    },
    /// Показать все сообщения всех крейтов.
    Messages,
    /// Показать справку.
    Help,
    /// Такого аргумента нет: имя аргумента.
    Unknown {
        /// Что пришло в командной строке.
        arg: String,
    },
}

/// Разобрать аргументы командной строки.
///
/// Аргумент, который не команда и не файл, — это ошибка: молча открывать
/// файл с именем `--что-то` хуже, чем сказать, что такого имени нет.
pub fn parse(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        Some("messages") => Command::Messages,
        Some("help") | Some("--help") | Some("-h") => Command::Help,
        None => Command::Run { path: None },
        Some(path) if path.starts_with('-') => Command::Unknown {
            arg: path.to_string(),
        },
        Some(path) => Command::Run {
            path: Some(std::path::PathBuf::from(path)),
        },
    }
}

/// Вывести все сообщения всех крейтов.
///
/// Нужны двум: починить текст и показать переводчику, что вообще есть.
/// Каждое сообщение выводится с настоящими значениями вместо имён в
/// фигурных скобках, иначе переводчик не поймёт, что подставляется.
pub fn print_messages(out: &mut impl Write) -> std::io::Result<()> {
    let crates: [(&str, Vec<(&str, String)>); 7] = [
        ("claxis-buffer", buffer::messages::samples()),
        ("claxis-disk", disk::messages::samples()),
        ("claxis-term", term::messages::samples()),
        ("claxis-i18n", i18n::messages::samples()),
        ("claxis-config", config::messages::samples()),
        ("Claxis", crate::messages::samples()),
        ("Claxis (main)", main_messages::samples()),
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
    // Описания команд — из каталога: пользователю с переводом английский текст
    // в справке читаться не должен.
    for (name, key) in COMMANDS {
        writeln!(out, "{}", m.command_line(name, &what(m, key)))?;
    }
    Ok(())
}

/// Команды редактора: имя и ключ каталога с описанием.
///
/// Имя — код, описание — перевод. Само описание в коде не держится: ключ
/// достаётся у каталога, а английский вариант лежит там же.
pub const COMMANDS: [(&str, What); 2] = [("messages", What::Messages), ("help", What::Help)];

/// Описание команды: какое слово из каталога подставить.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// Описание команды `messages`.
    Messages,
    /// Описание команды `help`.
    Help,
}

/// Текст описания команды на языке каталога.
pub fn what(m: &dyn main_messages::Messages, what: What) -> String {
    match what {
        What::Messages => m.what_messages(),
        What::Help => m.what_help(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{i18n, term};

    fn out_of(command: Command) -> String {
        use crate::main_messages::En;
        let mut buf: Vec<u8> = Vec::new();
        match command {
            Command::Messages => print_messages(&mut buf).unwrap(),
            Command::Help => print_help(&mut buf, &En).unwrap(),
            Command::Run { .. } | Command::Unknown { .. } => {}
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn every_binary_message_has_a_sample() {
        // Каталог строится по примерам. Сообщения, для которого примера нет,
        // переводчик не увидит никогда — и оно останется английским навсегда.
        let expected = [
            "store_failed",
            "read_file_failed",
            "buffer_error",
            "config_dir_failed",
            "no_home_dir",
            "snapshot_failed",
            "not_watched",
            "no_usable_config",
            "file_too_large",
        ];
        let samples: Vec<&str> = crate::messages::samples()
            .iter()
            .map(|(key, _)| *key)
            .collect();
        for key in expected {
            assert!(samples.contains(&key), "нет примера для {key}");
        }

        let expected = [
            "terminal_failed",
            "session_failed",
            "event_line",
            "using_last_good",
            "changed_outside",
            "unknown_argument",
            "usage",
            "opens_editor",
            "commands_heading",
            "command_line",
            "what_messages",
            "what_help",
        ];
        let samples: Vec<&str> = main_messages::samples()
            .iter()
            .map(|(key, _)| *key)
            .collect();
        for key in expected {
            assert!(samples.contains(&key), "нет примера для {key}");
        }
    }

    #[test]
    fn every_sample_key_is_unique() {
        // Повтор ключа в списке переводов означает, что переводчик правит одно и
        // то же дважды и не понимает, куда.
        for (name, list) in [
            ("Claxis", crate::messages::samples()),
            ("Claxis (main)", main_messages::samples()),
        ] {
            let mut seen: Vec<&str> = list.iter().map(|(key, _)| *key).collect();
            let total = seen.len();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), total, "в {name} повторяются ключи");
        }
    }

    #[test]
    fn arguments_are_parsed() {
        assert_eq!(parse(&[]), Command::Run { path: None });
        // Неизвестное слово — это файл, а не ошибка.
        assert_eq!(
            parse(&["file.rs".to_string()]),
            Command::Run {
                path: Some(std::path::PathBuf::from("file.rs"))
            }
        );
        assert_eq!(parse(&["messages".to_string()]), Command::Messages);
        assert_eq!(parse(&["--help".to_string()]), Command::Help);
        // Неизвестный ключ — не имя файла: иначе редактор молча откроет файл,
        // которого никто не просил.
        assert_eq!(
            parse(&["--что-то".to_string()]),
            Command::Unknown {
                arg: "--что-то".to_string()
            }
        );
        assert_eq!(
            parse(&["-x".to_string()]),
            Command::Unknown {
                arg: "-x".to_string()
            }
        );
    }

    #[test]
    fn every_crate_appears_in_the_message_list() {
        // Если крейт забыли, его сообщения нечем будет починить.
        let text = out_of(Command::Messages);
        for crate_name in [
            "claxis-buffer",
            "claxis-disk",
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
        assert!(!disk::messages::samples().is_empty());
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
            fn changed_outside(&self, p: &str) -> String {
                format!("файл {p} меняли снаружи")
            }
            fn unknown_argument(&self, a: &str) -> String {
                format!("неизвестный аргумент: {a}")
            }
            fn what_messages(&self) -> String {
                "показать все сообщения".to_string()
            }
            fn what_help(&self) -> String {
                "эта справка".to_string()
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

//! Реестр команд и разбор командной строки.

use crate::error::Error;
use crate::name::CommandName;

/// Реестр известных команд.
///
/// Реестр ничего не выполняет: он знает, какие имена существуют, чтобы
/// автодополнение показывало реальные варианты, а привязка не указывала в
/// пустоту. Проверка имени при разборе алиаса и хоткея идёт по нему.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    /// Зарегистрированные имена.
    names: Vec<CommandName>,
}

impl Registry {
    /// Пустой реестр.
    pub fn new() -> Self {
        Self::default()
    }

    /// Реестр со встроенными командами.
    pub fn std() -> Self {
        let mut registry = Self::new();
        for name in std_names() {
            registry
                .register(name)
                .expect("встроенное имя всегда корректно");
        }
        registry
    }

    /// Зарегистрировать команду. Повторная регистрация того же имени не
    /// меняет реестр: дважды зарегистрировать команду нельзя, но и не
    /// сломать это не повод.
    pub fn register(&mut self, name: CommandName) -> Result<(), Error> {
        if self.names.contains(&name) {
            return Ok(());
        }
        self.names.push(name);
        self.names.sort_by(|a, b| a.parts().cmp(b.parts()));
        Ok(())
    }

    /// Есть ли такая команда.
    pub fn has(&self, name: &CommandName) -> bool {
        self.names.contains(name)
    }

    /// Найти команду по имени.
    pub fn get(&self, name: &str) -> Result<&CommandName, Error> {
        let wanted = CommandName::parse(name)?;
        if self.names.contains(&wanted) {
            Ok(&self.names[self.names.iter().position(|n| n == &wanted).unwrap()])
        } else {
            Err(Error::UnknownCommand {
                name: name.to_string(),
            })
        }
    }

    /// Все зарегистрированные имена.
    pub fn names(&self) -> &[CommandName] {
        &self.names
    }

    /// Продолжения по префиксу — автодополнение.
    ///
    /// Пустой префикс даёт пространства имён: сначала `std` и плагины, дальше
    /// ничего. Префикс, под который не подошло ни одной команды, даёт
    /// пустой список, а не ошибку: пользователь просто дописывает.
    pub fn complete(&self, prefix: &[&str]) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .names
            .iter()
            .filter_map(|n| n.completions(prefix))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Единственное продолжение под префикс — его можно дописать сразу.
    pub fn complete_one(&self, prefix: &[&str]) -> Option<&str> {
        let mut all = self.complete(prefix);
        if all.len() == 1 { all.pop() } else { None }
    }
}

/// Цифры в начале строки — это повтор, а не имя.
fn leading_digits(text: &str) -> &str {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    &text[..end]
}

/// Имена встроенных команд.
fn std_names() -> Vec<CommandName> {
    [
        ":std::edit::move::up",
        ":std::edit::move::down",
        ":std::edit::move::left",
        ":std::edit::move::right",
        ":std::edit::move::word_start",
        ":std::edit::move::line_start",
        ":std::edit::move::line_end",
        ":std::edit::move::doc_start",
        ":std::edit::delete::line",
        ":std::edit::delete::word",
        ":std::edit::copy::line",
        ":std::edit::undo",
        ":std::edit::redo",
        ":std::edit::paste",
        ":std::file::save",
        ":std::file::open",
        ":std::file::close",
        ":std::mode::command",
        ":std::mode::edit",
        ":std::mode::insert",
        ":std::search::next",
        ":std::search::previous",
        ":std::macro::start",
        ":std::macro::stop",
        ":std::macro::replay",
    ]
    .map(|name| CommandName::parse(name).expect("встроенное имя всегда корректно"))
    .to_vec()
}

/// Разобранная командная строка: имя, число повторов, аргументы.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommandLine {
    /// Имя команды.
    pub name: CommandName,
    /// Число повторов. `None` — повтор не задан, значит один.
    pub repeat: Option<u32>,
    /// Аргументы после числа повторов.
    pub args: Vec<String>,
}

impl CommandLine {
    /// Разобрать строку вида `3:std::edit::move::word::right path`.
    ///
    /// Порядок задан документацией: сначала число повторов, потом имя, потом
    /// аргументы. Число пишется перед двоеточием — так же, как в vim, где
    /// `5dd` идёт быстрее, чем `:delete 5 lines`.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let digits = leading_digits(text);
        let rest = &text[digits.len()..];
        let mut parts = rest.split_whitespace();
        let first = parts.next().ok_or_else(|| Error::BadCommandName {
            got: text.to_string(),
            reason: "empty".to_string(),
        })?;
        let name = CommandName::parse(first)?;
        let repeat = if digits.is_empty() {
            None
        } else {
            Some(digits.parse().map_err(|_| Error::BadRepeatCount {
                got: digits.to_string(),
            })?)
        };
        Ok(Self {
            name,
            repeat,
            args: parts.map(str::to_string).collect(),
        })
    }

    /// Сколько раз выполнить: заданное повторение или один раз.
    pub fn times(&self) -> u32 {
        self.repeat.unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> CommandName {
        CommandName::parse(text).unwrap()
    }

    #[test]
    fn std_commands_are_registered_and_sorted() {
        let registry = Registry::std();
        assert!(registry.has(&CommandName::parse(":std::file::save").unwrap()));
        assert!(!registry.has(&CommandName::parse(":std::file::save::all").unwrap()));
        let sorted = registry.names();
        assert!(sorted.windows(2).all(|w| w[0].parts() <= w[1].parts()));
    }

    #[test]
    fn an_unknown_command_is_reported_not_silently_dropped() {
        let registry = Registry::std();
        assert!(matches!(
            registry.get(":std::file::save::everything"),
            Err(Error::UnknownCommand { .. })
        ));
    }

    #[test]
    fn registering_the_same_name_twice_keeps_one_entry() {
        let mut registry = Registry::new();
        let name = CommandName::parse(":std::file::save").unwrap();
        registry.register(name.clone()).unwrap();
        registry.register(name.clone()).unwrap();
        assert_eq!(registry.names().len(), 1);
    }

    #[test]
    fn a_plugin_lands_next_to_std_in_the_same_registry() {
        let mut registry = Registry::std();
        registry
            .register(CommandName::parse(":my-plugin::file::save").unwrap())
            .unwrap();
        assert_eq!(
            registry.complete(&[]),
            vec!["my-plugin", "std"],
            "пространства предлагаются вместе"
        );
    }

    #[test]
    fn completion_walks_the_hierarchy_level_by_level() {
        let registry = Registry::std();
        assert_eq!(
            registry.complete(&["std"]),
            vec!["edit", "file", "macro", "mode", "search"]
        );
        assert_eq!(
            registry.complete(&["std", "edit"]),
            vec!["copy", "delete", "move", "paste", "redo", "undo"]
        );
        assert_eq!(
            registry.complete(&["std", "edit", "move"]),
            vec![
                "doc_start",
                "down",
                "left",
                "line_end",
                "line_start",
                "right",
                "up",
                "word_start",
            ]
        );
    }

    #[test]
    fn a_unique_completion_is_finished_right_away() {
        let mut registry = Registry::new();
        registry.register(name(":a::one")).unwrap();
        registry.register(name(":a::two")).unwrap();
        assert_eq!(registry.complete_one(&["a"]), None, "one и two — оба");
        registry.register(name(":a::three::only")).unwrap();
        assert_eq!(
            registry.complete_one(&["a", "three"]),
            Some("only"),
            "единственное продолжение дописывается сразу"
        );
        assert_eq!(
            registry.complete(&["a"]),
            vec!["one", "three", "two"],
            "все продолжения подряд, без повторов"
        );
    }

    #[test]
    fn a_fully_typed_name_has_nothing_to_add() {
        let registry = Registry::std();
        assert_eq!(
            registry.complete(&["std", "edit", "move", "up"]),
            Vec::<&str>::new(),
            "имя дописано целиком, добавлять нечего"
        );
        assert_eq!(registry.complete_one(&["std", "edit", "move", "up"]), None);
    }

    #[test]
    fn completion_for_a_dead_prefix_is_empty_and_not_an_error() {
        let registry = Registry::std();
        assert!(registry.complete(&["nope"]).is_empty());
        assert_eq!(registry.complete_one(&["nope"]), None);
    }

    #[test]
    fn a_count_goes_before_the_name_like_in_vim() {
        let line = CommandLine::parse("3:std::edit::move::word::right").unwrap();
        assert_eq!(line.name.to_string(), ":std::edit::move::word::right");
        assert_eq!(line.repeat, Some(3));
        assert_eq!(line.times(), 3);
        assert!(line.args.is_empty());
    }

    #[test]
    fn a_command_line_without_a_count_runs_once() {
        let line = CommandLine::parse(":std::file::save").unwrap();
        assert_eq!(line.repeat, None);
        assert_eq!(line.times(), 1);
    }

    #[test]
    fn a_multi_digit_count_before_the_name_is_read_as_one_number() {
        let line = CommandLine::parse("100:std::edit::delete::line").unwrap();
        assert_eq!(line.repeat, Some(100));
    }

    #[test]
    fn arguments_follow_the_name() {
        let line = CommandLine::parse(":std::file::open path/to/file").unwrap();
        assert_eq!(line.repeat, None);
        assert_eq!(line.args, vec!["path/to/file"]);
        let repeated = CommandLine::parse("2:std::file::open path/to/file").unwrap();
        assert_eq!(repeated.repeat, Some(2));
        assert_eq!(repeated.args, vec!["path/to/file"]);
    }

    #[test]
    fn a_number_after_the_name_is_an_argument_not_a_count() {
        // Порядок один: повтор до имени. Всё, что после имени, — аргументы,
        // даже если это число: иначе `открыть файл 2024` не отличить от
        // повтора.
        let line = CommandLine::parse(":std::file::open 2024").unwrap();
        assert_eq!(line.repeat, None);
        assert_eq!(line.args, vec!["2024"]);
    }

    #[test]
    fn a_count_too_large_to_be_one_is_an_error_not_a_wraparound() {
        let error = CommandLine::parse("99999999999999999999:std::file::save").unwrap_err();
        assert!(matches!(error, Error::BadRepeatCount { .. }));
    }

    #[test]
    fn a_broken_command_line_is_an_error_not_an_empty_one() {
        assert!(CommandLine::parse("").is_err());
        assert!(CommandLine::parse("std::file::save").is_err());
        assert!(CommandLine::parse(":std::::save").is_err());
        assert!(CommandLine::parse("5").is_err(), "повтор без команды");
    }
}

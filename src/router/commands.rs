//! Сборка реестра команд.
//!
//! Каждый крейт объявляет свои команды у себя, в файле `commands.rs`, и
//! приносит сюда список имён. Собираются они под пространством имён `std`,
//! поэтому `edit::undo` из `claxis-rope` становится `:std::edit::undo`.
//!
//! Выполняются команды редактором. Здесь только реестр: какие имена
//! существуют и что на что разворачивается.
//!
//! Список крейтов дописывается здесь, а не внутри `claxis-commands`: там чужих
//! команд нет и знать о них нечего.

use crate::api::commands::{Error, Registry};
use crate::api::rope::COMMANDS;

/// Объявления крейтов: список имён из каждого `commands.rs`.
const DECLARED: &[(&str, &[&str])] = &[("std", COMMANDS)];

/// Реестр всех команд редактора.
///
/// Ошибка сборки возвращается, а не проглатывается: опечатка в объявлении
/// должна быть видна при запуске, а не всплыть в момент нажатия.
pub fn registry() -> Result<Registry, Error> {
    Registry::from_parts(DECLARED)
}

/// Команда по имени, как её пишет человек.
#[cfg(test)]
fn find(name: &str) -> Result<crate::api::commands::CommandName, Error> {
    registry()?.get(name).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crate_command_is_registered_under_its_own_name() {
        assert_eq!(
            find(":std::edit::undo").unwrap().to_string(),
            ":std::edit::undo"
        );
    }

    #[test]
    fn every_declared_command_lands_in_the_registry() {
        let registry = registry().unwrap();
        for (namespace, commands) in DECLARED {
            for command in *commands {
                let name = format!(":{namespace}::{command}");
                assert!(
                    registry.get(&name).is_ok(),
                    "команда {name} не зарегистрирована"
                );
            }
        }
    }

    #[test]
    fn a_command_that_no_crate_declared_is_not_invented() {
        // Реестр не отвечает на всё подряд: объявить команду должен крейт.
        assert!(find(":std::file::save").is_err());
        assert!(find(":std::edit::move::up").is_err());
    }

    #[test]
    fn completion_walks_the_namespaces_of_real_commands() {
        let registry = registry().unwrap();
        assert_eq!(registry.complete(&[]), vec!["std"]);
        assert_eq!(registry.complete(&["std", "edit"]), vec!["redo", "undo"]);
    }
}

//! Где лежит конфиг на диске.
//!
//! Конфиг пользователя — в каталоге конфигурации операционной системы:
//! `$XDG_CONFIG_HOME/claxis` или `~/.config/claxis`. Это правило XDG, чтобы
//! файлы нашлись там, где их ищут другие программы.

use std::path::{Path, PathBuf};

/// Имя каталога внутри `XDG_CONFIG_HOME`.
pub const DIR_NAME: &str = "claxis";

/// Каталог конфига пользователя.
///
/// `$XDG_CONFIG_HOME/claxis`, а если переменной нет — `~/.config/claxis`.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        && !base.is_empty()
    {
        return Some(PathBuf::from(base).join(DIR_NAME));
    }
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config").join(DIR_NAME))
}

/// Папка с темами внутри каталога конфига.
pub fn theme_dir(config: impl AsRef<Path>) -> PathBuf {
    config.as_ref().join(crate::schema::THEME_DIR)
}

/// Папка с переводами внутри каталога конфига.
pub fn lang_dir(config: impl AsRef<Path>) -> PathBuf {
    config.as_ref().join(crate::schema::LANG_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_and_languages_live_in_folders() {
        // Тема и перевод — файлы в папках, имя файла значит название темы или
        // код языка.
        let config = Path::new("/home/u/.config/claxis");
        assert_eq!(theme_dir(config), Path::new("/home/u/.config/claxis/theme"));
        assert_eq!(lang_dir(config), Path::new("/home/u/.config/claxis/lang"));
    }

    #[test]
    fn config_dir_is_inside_xdg_config() {
        // Не в домашней папке напрямую: там живут документы, а не настройки.
        let dir = user_dir().expect("каталог должен находиться");
        assert!(dir.ends_with(DIR_NAME));
    }

    #[test]
    fn an_empty_xdg_config_home_is_not_a_directory() {
        // Пустая переменная — это не путь: иначе каталог оказался бы в
        // текущей директории и потерялся при смене папки.
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: тест единственный, кто трогает переменную, и снимает её сразу.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "") };
        let dir = user_dir();
        match previous {
            Some(value) => unsafe { std::env::set_var("XDG_CONFIG_HOME", value) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let dir = dir.expect("пустая переменная не должна ломать поиск");
        assert!(!dir.as_os_str().is_empty(), "получился пустой путь конфига");
        assert!(dir.ends_with(DIR_NAME));
    }

    #[test]
    fn theme_and_lang_folders_are_separate() {
        // Темы и переводы не должны слипаться: это разные вещи, и папки у
        // них разные.
        let config = Path::new("/cfg");
        assert_ne!(theme_dir(config), lang_dir(config));
        assert!(theme_dir(config).starts_with(config));
        assert!(lang_dir(config).starts_with(config));
    }

    #[test]
    fn folder_names_match_the_schema() {
        // Имена папок объявлены в схеме: путь и схема не должны разойтись.
        assert_eq!(
            theme_dir("/cfg"),
            Path::new("/cfg").join(crate::schema::THEME_DIR)
        );
        assert_eq!(
            lang_dir("/cfg"),
            Path::new("/cfg").join(crate::schema::LANG_DIR)
        );
    }
}

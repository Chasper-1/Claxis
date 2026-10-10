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

/// Каталог конфига в заданном месте — нужно в тестах.
pub fn dir_at(base: impl AsRef<Path>) -> PathBuf {
    base.as_ref().to_path_buf()
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
}

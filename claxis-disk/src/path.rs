//! Где лежит кеш на диске.
//!
//! Кеш — это то, что можно удалить и потерять без вреда для текста: снапшоты
//! истории, последний корректный конфиг. Раскладку пользовательских файлов
//! крейт не знает: где лежит конфиг — дело `claxis-config`.

use std::path::{Path, PathBuf};

/// Пути хранилища.
#[derive(Debug, Clone)]
pub struct StorePaths {
    /// Каталог кеша редактора, например `~/.cache/claxis`.
    pub dir: PathBuf,
}

impl StorePaths {
    /// Путь по умолчанию: `$XDG_CACHE_HOME/claxis`, иначе `~/.cache/claxis`.
    pub fn from_env() -> Self {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            dir: base.join(crate::defaults::CACHE_DIR),
        }
    }

    /// Путь для заданного каталога — нужно в тестах и для нестандартного
    /// размещения.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Файл базы.
    pub fn database(&self) -> PathBuf {
        self.dir.join("store.db")
    }

    /// Создать каталог, если его нет.
    pub fn ensure_dir(&self) -> crate::Result<()> {
        crate::file::create_dir(&self.dir)
    }

    /// Каталог существует.
    pub fn exists(&self) -> bool {
        Path::new(&self.dir).is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каталог под тест, удаляемый вместе с собой.
    struct Temp {
        dir: PathBuf,
    }

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-disk-path-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            Self { dir }
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn the_default_path_lives_in_the_cache_and_ends_with_the_editor_name() {
        let paths = StorePaths::from_env();
        let dir = paths.dir.display().to_string();
        assert!(dir.ends_with("claxis"), "{dir}");
        assert!(dir.contains("cache") || dir.contains("Cache"), "{dir}");
        assert!(paths.database().display().to_string().ends_with("store.db"));
    }

    #[test]
    fn an_empty_xdg_cache_home_is_not_a_directory() {
        // Пустая переменная — это не путь: иначе кеш оказался бы в текущем
        // каталоге и потерялся при смене директории.
        let previous = std::env::var_os("XDG_CACHE_HOME");
        // SAFETY: тест единственный, кто трогает переменную, и снимает её сразу.
        unsafe { std::env::set_var("XDG_CACHE_HOME", "") };
        let paths = StorePaths::from_env();
        match previous {
            Some(value) => unsafe { std::env::set_var("XDG_CACHE_HOME", value) },
            None => unsafe { std::env::remove_var("XDG_CACHE_HOME") },
        }
        assert!(
            !paths.dir.display().to_string().starts_with("claxis/claxis"),
            "путь получился относительным: {}",
            paths.dir.display()
        );
    }

    #[test]
    fn the_database_lives_inside_the_directory() {
        let paths = StorePaths::new("/tmp/каталог");
        assert_eq!(paths.database(), Path::new("/tmp/каталог").join("store.db"));
        assert!(paths.database().starts_with(&paths.dir));
    }

    #[test]
    fn ensure_dir_creates_the_whole_path_and_is_idempotent() {
        let temp = Temp::new("ensure");
        let paths = StorePaths::new(temp.dir.join("глубоко").join("вложенно"));
        assert!(!paths.exists());
        paths.ensure_dir().unwrap();
        assert!(paths.exists());
        paths.ensure_dir().unwrap();
        assert!(paths.exists());
    }

    #[test]
    fn ensure_dir_reports_what_it_could_not_do() {
        let temp = Temp::new("ensure-error");
        let blocker = temp.dir.join("не-каталог");
        std::fs::create_dir_all(&temp.dir).unwrap();
        std::fs::write(&blocker, "файл".as_bytes()).unwrap();

        let paths = StorePaths::new(blocker.join("внутрь"));
        let error = paths.ensure_dir().unwrap_err();
        assert!(error.text().contains("внутрь"), "{error}");
    }

    #[test]
    fn exists_is_false_before_and_true_after() {
        let temp = Temp::new("exists");
        let paths = StorePaths::new(&temp.dir);
        assert!(!paths.exists());
        paths.ensure_dir().unwrap();
        assert!(paths.exists());
    }
}

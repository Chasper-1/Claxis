use std::path::{Path, PathBuf};

/// Где лежит хранилище.
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
            dir: base.join("claxis"),
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
        std::fs::create_dir_all(&self.dir).map_err(|e| crate::Error::PrepareDir {
            path: self.dir.display().to_string(),
            reason: e.to_string(),
        })
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
            let dir = std::env::temp_dir().join(format!("claxis-store-path-{name}"));
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
    fn store_paths_from_env_have_claxis_dir() {
        // Путь по умолчанию заканчивается именем редактора.
        let paths = StorePaths::from_env();
        let dir = paths.dir.display().to_string();
        assert!(
            dir.ends_with("claxis"),
            "путь должен кончаться на claxis: {dir}"
        );
        assert!(paths.database().display().to_string().ends_with("store.db"));
    }

    #[test]
    fn ensure_dir_creates_the_whole_path_and_is_idempotent() {
        let temp = Temp::new("ensure-dir");
        let paths = StorePaths::new(temp.dir.join("глубоко").join("вложенно"));
        assert!(!paths.exists());
        paths.ensure_dir().unwrap();
        assert!(paths.exists());
        paths.ensure_dir().unwrap();
        assert!(paths.exists());
    }

    #[test]
    fn ensure_dir_says_what_it_could_not_do() {
        // Ошибка подготовки каталога не теряется: без каталога база не откроется.
        let temp = Temp::new("ensure-dir-error");
        let blocker = temp.dir.join("не-каталог");
        std::fs::create_dir_all(&temp.dir).unwrap();
        std::fs::write(&blocker, "файл".as_bytes()).unwrap();

        let paths = StorePaths::new(blocker.join("внутрь"));
        let error = paths.ensure_dir().unwrap_err();
        assert!(
            error.to_string().contains("внутрь"),
            "в ошибке нет пути: {error}"
        );
    }

    #[test]
    fn store_paths_place_the_database_inside_the_dir() {
        let paths = StorePaths::new("/tmp/каталог");
        assert_eq!(paths.database(), Path::new("/tmp/каталог").join("store.db"));
        assert!(paths.database().starts_with(&paths.dir));
    }

    #[test]
    fn an_empty_xdg_cache_home_is_not_used() {
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
        assert!(paths.dir.ends_with("claxis"));
        assert!(
            !paths.dir.display().to_string().starts_with("claxis/claxis"),
            "путь получился относительным: {}",
            paths.dir.display()
        );
    }
}

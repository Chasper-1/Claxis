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

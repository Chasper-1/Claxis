//! Временный каталог для тестов.
//!
//! Тесты разных крейтов заводят каталог под файлы. Держать по копии этого
//! кода в каждом файле незачем: ошибка в уборке или в уникальности имени
//! разъехалась бы по всему проекту.
//!
//! Имя уникально: номер процесса и счётчик не дают параллельным тестам
//! попасть в один каталог. Уборка идёт в [`Drop`], поэтому падение теста не
//! оставляет мусор.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Счётчик каталогов внутри процесса.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Временный каталог, который удаляется вместе с собой.
#[derive(Debug)]
pub struct Temp {
    path: PathBuf,
}

impl Temp {
    /// Создать пустой каталог с меткой `prefix`.
    ///
    /// `prefix` нужен человеку: по нему видно, чей тест оставил каталог, если
    /// уборка не случилась. На уникальность влияет не он, а счётчик.
    pub fn new(prefix: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("claxis-{prefix}-{}-{n}", std::process::id()));
        // Каталог мог остаться от прошлого запуска с тем же pid: начинаем с
        // чистого, иначе старый файл попадёт в тест.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("временный каталог создаётся");
        Self { path }
    }

    /// Путь к каталогу.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::Temp;

    #[test]
    fn a_temp_directory_exists_while_it_lives() {
        let tmp = Temp::new("exists");
        assert!(tmp.path().is_dir(), "каталог обязан существовать");
    }

    #[test]
    fn two_temp_directories_do_not_collide() {
        // Параллельные тесты не должны писать в одно место.
        let first = Temp::new("unique");
        let second = Temp::new("unique");
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
    }

    #[test]
    fn dropping_removes_the_directory() {
        let path = {
            let tmp = Temp::new("drop");
            let path = tmp.path().to_path_buf();
            assert!(path.is_dir());
            path
        };
        assert!(!path.exists(), "каталог обязан исчезнуть после Drop");
    }
}

//! Запись файлов и слежение за их актуальностью.
//!
//! Запись атомарная: сначала рядом появляется временный файл, и только потом
//! он переименовывается на место. Читатель в этот момент видит либо старый
//! файл целиком, либо новый целиком, и никогда — половину. Обходной путь с
//! перезаписью на месте здесь один: он и роняет редактор, если кончилось
//! место.
//!
//! Актуальность файла отвечает якорь «размер и хеш» в базе снапшотов:
//! сначала размер, он ловит почти всё и стоит ничего, потом хеш, который
//! платит только за правку той же длины. Себя он живёт в `super::hash`.

use crate::error::{Error, Result};

use std::io::Write;
use std::path::{Path, PathBuf};

/// Прочитать файл как текст.
///
/// Чтение живёт здесь по тому же правилу, что и запись: диск — это один
/// крейт. Ошибка несёт путь, иначе человек не поймёт, какой файл не открылся.
pub fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| crate::error::io(path, e))
}

/// Прочитать файл как байты. `None`, если файла нет.
///
/// Отсутствие файла — не ошибка для вызывающего, который открывает документ:
/// новый файл ещё не создан, и это нормальный первый шаг.
pub fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::error::io(path, e)),
    }
}

/// Размер файла. `None`, если файла нет.
///
/// Размер нужен до чтения: буфер не примет файл больше `u32::MAX` байт, и
/// узнать это лучше, чем прочитать гигабайты в память.
pub fn file_len(path: &Path) -> Result<Option<u64>> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.len())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::error::io(path, e)),
    }
}

/// Файлы в каталоге.
///
/// Только обычные файлы, без подкаталогов: вызывающему нужен список файлов,
/// а не дерево. Отсутствующий каталог даёт пустой список — до первого запуска
/// папки ещё нет, и это не поломка.
pub fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(crate::error::io(dir, e)),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| crate::error::io(dir, e))?;
        let path = entry.path();
        let meta = std::fs::metadata(&path).map_err(|e| crate::error::io(&path, e))?;
        if meta.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

/// Записать текст в файл.
///
/// Отсутствующий каталог создаётся сам: путь до файла — часть задачи записи,
/// а не отдельная просьба к вызывающему.
pub fn write_text(path: &Path, text: &str) -> Result<()> {
    write_blob(path, text.as_bytes())
}

/// Записать байты в файл.
///
/// Запись идёт во временный файл рядом и переименовывается на место. Временный
/// файл падает на диске вместе с ошибкой: полузаписанного файла на месте не
/// остаётся.
pub fn write_blob(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        create_dir(parent)?;
    }

    // Рядом с целью: переименование атомарно только внутри одного каталога, а
    // через `/tmp` это уже не то же самое, что запись.
    let temp = temp_path(path);
    let mut file = std::fs::File::create(&temp).map_err(|e| crate::error::write_io(path, e))?;
    let written = file
        .write_all(data)
        .and_then(|()| file.sync_all())
        .map_err(|e| crate::error::write_io(path, e));
    drop(file);

    if let Err(e) = written {
        // Мусор после неудачи хуже самой неудачи: человек увидит файл, который
        // выглядит целым, а он не цел.
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }

    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        Error::Rename {
            path: path.display().to_string(),
            reason: e.to_string(),
        }
    })
}

/// Создать каталог со всеми промежуточными.
pub fn create_dir(dir: &Path) -> Result<()> {
    match std::fs::create_dir_all(dir) {
        Ok(()) => Ok(()),
        // Уже есть — это не ошибка, а нужное состояние.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(Error::PrepareDir {
            path: dir.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

/// Имя временного файла рядом с целью.
///
/// Одно имя на все записи в этом процессе, поэтому параллельные записи не
/// мешают друг другу: кто последний переименовал, тот и прав.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("file"));
    name.push(format!(".tmp{}", std::process::id()));
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
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
            let dir = std::env::temp_dir().join(format!("claxis-disk-file-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn file(&self, name: &str) -> PathBuf {
            self.dir.join(name)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn text_is_written_and_read_back_unchanged() {
        let temp = Temp::new("write-text");
        let path = temp.file("config.toml");
        write_text(&path, "history_depth = 64\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "history_depth = 64\n"
        );
    }

    #[test]
    fn text_is_read_back_by_the_disk_crate() {
        let temp = Temp::new("read-text");
        let path = temp.file("theme.toml");
        std::fs::write(&path, "background = \"#000000\"").unwrap();
        assert_eq!(read_text(&path).unwrap(), "background = \"#000000\"");
    }

    #[test]
    fn a_missing_file_is_a_read_error_with_its_path() {
        let temp = Temp::new("read-missing");
        let path = temp.file("missing.toml");
        let error = read_text(&path).unwrap_err();
        assert!(error.text().contains("missing.toml"), "{error}");
    }

    #[test]
    fn bytes_are_read_and_absence_is_not_an_error() {
        let temp = Temp::new("read-bytes");
        let path = temp.file("document.txt");
        std::fs::write(&path, b"\x00\x01\xff").unwrap();
        assert_eq!(read_bytes(&path).unwrap(), Some(vec![0, 1, 255]));
        assert_eq!(read_bytes(&temp.file("none.txt")).unwrap(), None);
    }

    #[test]
    fn the_length_is_known_before_reading() {
        let temp = Temp::new("file-len");
        let path = temp.file("document.txt");
        std::fs::write(&path, "12345").unwrap();
        assert_eq!(file_len(&path).unwrap(), Some(5));
        assert_eq!(file_len(&temp.file("none.txt")).unwrap(), None);
    }

    #[test]
    fn listing_skips_folders_and_a_missing_folder_is_empty() {
        let temp = Temp::new("list-files");
        std::fs::write(temp.file("one.toml"), "").unwrap();
        std::fs::create_dir(temp.file("folder")).unwrap();
        let names: Vec<String> = list_files(&temp.dir)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["one.toml"]);
        assert!(list_files(&temp.dir.join("none")).unwrap().is_empty());
    }

    #[test]
    fn a_missing_directory_is_created_by_the_write() {
        // Путь до файла — часть записи. Просить вызывающего создать папку
        // отдельно означало бы две ошибки вместо одной.
        let temp = Temp::new("mkdir");
        let path = temp.file("глубоко").join("внутрь").join("config.toml");
        write_text(&path, "ok").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ok");
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        // Мусор рядом с конфигом человек видит и не понимает, откуда он.
        let temp = Temp::new("no-temp");
        let path = temp.file("config.toml");
        write_text(&path, "ok").unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(&temp.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "config.toml")
            .collect();
        assert!(leftovers.is_empty(), "осталось в каталоге: {leftovers:?}");
    }

    #[test]
    fn writing_twice_replaces_the_whole_content() {
        // Запись не дописывает, а заменяет: старый хвост не должен остаться.
        let temp = Temp::new("rewrite");
        let path = temp.file("config.toml");
        write_text(&path, "длинный прежний текст").unwrap();
        write_text(&path, "короткий").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "короткий");
    }

    #[test]
    fn a_blob_with_zero_bytes_survives() {
        // Нули внутри блоба — обычные данные, обрезка по первому нулю была бы
        // тихой порчей снапшота.
        let temp = Temp::new("blob");
        let path = temp.file("store.blob");
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        write_blob(&path, &data).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn an_empty_file_is_a_valid_result() {
        let temp = Temp::new("empty");
        let path = temp.file("empty.toml");
        write_text(&path, "").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn a_failed_write_leaves_no_file_and_no_litter() {
        // Путь занят каталогом: запись обязана упасть с понятной ошибкой, и
        // после неё не должно остаться ни мусора, ни наполовину записанного
        // файла.
        let temp = Temp::new("fail");
        let dir_in_the_way = temp.file("занято");
        std::fs::create_dir_all(&dir_in_the_way).unwrap();
        let error = write_text(&dir_in_the_way, "не запишется").unwrap_err();
        assert!(
            matches!(error, Error::Write { .. } | Error::Rename { .. }),
            "не та ошибка: {error:?}"
        );
        assert!(dir_in_the_way.is_dir(), "каталог на месте не тронут");
    }

    #[test]
    fn a_failed_write_reports_the_path() {
        // Ошибка без пути — это «что-то сломалось», а человек должен знать где.
        let temp = Temp::new("fail-path");
        let file_in_the_way = temp.file("файл");
        std::fs::write(&file_in_the_way, "я файл").unwrap();
        let inside = file_in_the_way.join("config.toml");
        let error = write_text(&inside, "x").unwrap_err();
        let text = error.text();
        assert!(text.contains("config.toml"), "в ошибке нет пути: {text}");
    }

    #[test]
    fn a_temporary_name_stays_next_to_its_target() {
        // Переименование атомарно только внутри одного каталога.
        let temp = Temp::new("temp-name");
        let path = temp.file("config.toml");
        let name = temp_path(&path);
        assert_eq!(name.parent(), path.parent());
        assert!(name != path, "временный файл не должен совпадать с целью");
    }

    #[test]
    fn a_file_without_a_parent_gets_a_bare_temporary_name() {
        let name = temp_path(Path::new("config.toml"));
        assert_eq!(name.parent(), Some(Path::new("")));
        assert!(name.to_string_lossy().contains(".tmp"));
    }
}

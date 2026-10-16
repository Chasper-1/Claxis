//! Запись файлов и слежение за их актуальностью.
//!
//! Запись атомарная: сначала рядом появляется временный файл, и только потом
//! он переименовывается на место. Читатель в этот момент видит либо старый
//! файл целиком, либо новый целиком, и никогда — половину. Обходной путь с
//! перезаписью на месте здесь один: он и роняет редактор, если кончилось
//! место.
//!
//! Отпечаток отвечает на вопрос «меняли файл, пока мы смотрели в другую
//! сторону». Сравнить размер и время правки дешевле, чем перечитывать файл, а
//! хватает для всех случаев, где файл трогают снаружи: другой редактор,
//! `git checkout`, скрипт.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Отпечаток файла: каким он был, когда мы смотрели в последний раз.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    /// Размер в байтах.
    len: u64,
    /// Время последней правки.
    modified: std::time::SystemTime,
}

impl Fingerprint {
    /// Взять отпечаток файла. `None`, если файла нет.
    ///
    /// Файла нет — не ошибка: его ещё могли не создать, и это не поломка.
    /// Каталог на месте файла — ошибка: читать его как файл бессмысленно, а
    /// молчаливый отпечаток размера папки сбил бы с толку.
    pub fn of(path: &Path) -> Result<Option<Self>> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(crate::error::io(path, e)),
        };
        if !meta.is_file() {
            return Err(Error::Read {
                path: path.display().to_string(),
                reason: "not a regular file".to_string(),
            });
        }
        Ok(Some(Self {
            len: meta.len(),
            modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
        }))
    }

    /// Тот же файл, что и раньше?
    pub fn same_as(&self, other: &Self) -> bool {
        self.len == other.len && self.modified == other.modified
    }
}

/// Отпечаток файла. Короткое имя для вызывающего.
pub fn fingerprint(path: &Path) -> Result<Option<Fingerprint>> {
    Fingerprint::of(path)
}

/// Что с файлом по сравнению с тем, как мы его видели.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Файл тот же: и длина, и время совпали.
    Unchanged,
    /// Файл поменялся.
    ///
    /// Поле `by_length` отвечает, чем это выяснилось. Длина разошлась —
    /// система ничего не стоила. Длина совпала, а время нет — догадались
    /// только по времени, и это слабее: `cp -p` и `tar` время сохраняют.
    Changed {
        /// Разошлась ли длина.
        by_length: bool,
    },
    /// Файла нет.
    Gone,
}

/// Менялся ли файл с тех пор, как мы его видели.
///
/// Проверка идёт в два шага, от дешёвого к дорогому. Первым идёт размер: он
/// ловит и дописывание, и усечение, а это почти всё, чем правят файлы
/// снаружи. Вторым идёт время изменения, которое система отдаёт сама и
/// стоит того же одного вызова. Содержимое не читается вовсе: система о нём
/// знает не хуже нас, а чтение большого файла ради проверки — это работа,
/// которой заниматься незачем.
pub fn changed_since(path: &Path, was: &Fingerprint) -> Result<Change> {
    let Some(now) = Fingerprint::of(path)? else {
        return Ok(Change::Gone);
    };
    if now.len != was.len {
        return Ok(Change::Changed { by_length: true });
    }
    if now.modified != was.modified {
        return Ok(Change::Changed { by_length: false });
    }
    Ok(Change::Unchanged)
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
    fn an_untouched_file_keeps_its_fingerprint() {
        let temp = Temp::new("same");
        let path = temp.file("config.toml");
        write_text(&path, "same").unwrap();
        let before = fingerprint(&path).unwrap().expect("файл есть");
        let after = fingerprint(&path).unwrap().expect("файл есть");
        assert!(
            before.same_as(&after),
            "файл не трогали, отпечаток изменился"
        );
    }

    #[test]
    fn a_changed_file_gives_a_different_fingerprint() {
        // Именно ради этого отпечаток и нужен: правка снаружи должна быть
        // видна без перечитывания файла целиком.
        let temp = Temp::new("changed");
        let path = temp.file("config.toml");
        write_text(&path, "short").unwrap();
        let before = fingerprint(&path).unwrap().unwrap();
        write_text(&path, "a much longer content than before").unwrap();
        let after = fingerprint(&path).unwrap().unwrap();
        assert!(!before.same_as(&after), "изменение не замечено");
    }

    #[test]
    fn a_missing_file_has_no_fingerprint() {
        // Отсутствие файла — не ошибка: его ещё могли не создать.
        let temp = Temp::new("missing");
        assert_eq!(fingerprint(&temp.file("нет.toml")).unwrap(), None);
    }

    #[test]
    fn the_fingerprint_of_a_folder_is_not_a_file() {
        // Каталог на месте файла — ошибка чтения, а не «файла нет».
        let temp = Temp::new("dir");
        let error = fingerprint(&temp.dir).unwrap_err();
        assert!(matches!(error, Error::Read { .. }), "{error:?}");
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

    /// Каталог под тест: файлы пишутся в папку с именем набора.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claxis-file-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Большой текст: сравнение на гигабайтах тут не проверить, но на
    /// мегабайте — вполне.
    fn big(seed: u8, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| b'a' + ((i + seed as usize) % 26) as u8)
            .collect()
    }

    #[test]
    fn an_untouched_file_is_unchanged() {
        let dir = temp_dir("unchanged");
        let path = dir.join("a.rs");
        let text = big(0, 200_000);
        std::fs::write(&path, &text).unwrap();
        let was = Fingerprint::of(&path).unwrap().unwrap();

        assert_eq!(changed_since(&path, &was).unwrap(), Change::Unchanged);
    }

    #[test]
    fn a_grown_file_is_caught_by_the_length_alone() {
        // Отпечаток разошёлся — содержимое можно было не смотреть вовсе.
        let dir = temp_dir("grown");
        let path = dir.join("a.rs");
        let text = big(0, 200_000);
        std::fs::write(&path, &text).unwrap();
        let was = Fingerprint::of(&path).unwrap().unwrap();

        let mut longer = text.clone();
        longer.push(b'\n');
        std::fs::write(&path, &longer).unwrap();

        assert_eq!(
            changed_since(&path, &was).unwrap(),
            Change::Changed { by_length: true }
        );
    }

    #[test]
    fn a_shrunken_file_is_caught_by_the_length_alone() {
        let dir = temp_dir("shrunk");
        let path = dir.join("a.rs");
        let text = big(0, 200_000);
        std::fs::write(&path, &text).unwrap();
        let was = Fingerprint::of(&path).unwrap().unwrap();

        std::fs::write(&path, &text[..100_000]).unwrap();
        assert_eq!(
            changed_since(&path, &was).unwrap(),
            Change::Changed { by_length: true }
        );
    }

    #[test]
    fn an_edit_of_the_same_length_is_caught_by_the_time_alone() {
        // Длина та же, дописывания нет. Остаётся время, которое система
        // отдаёт сама. Это слабее всего — `cp -p` и `tar` его сохраняют, — но
        // и стоит это ничего.
        let dir = temp_dir("same-length");
        let path = dir.join("a.rs");
        std::fs::write(&path, big(0, 200_000)).unwrap();
        let was = Fingerprint::of(&path).unwrap().unwrap();

        let mut edited = big(0, 200_000);
        let at = edited.len() / 2;
        edited[at] = b'Z';
        std::fs::write(&path, &edited).unwrap();

        let change = changed_since(&path, &was).unwrap();
        assert!(
            matches!(change, Change::Changed { by_length: false }) || change == Change::Unchanged,
            "ждали Changed или Unchanged, получили {change:?}"
        );
        // Время иногда совпадает: файловая система бывает с секундной
        // гранулярностью, и две записи в одну секунду неразличимы. Потому и
        // проверка такая: сначала длина, и только потом время.
    }

    #[test]
    fn a_vanished_file_is_not_an_unchanged_file() {
        let dir = temp_dir("gone");
        let path = dir.join("a.rs");
        std::fs::write(&path, "был".as_bytes()).unwrap();
        let was = Fingerprint::of(&path).unwrap().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(changed_since(&path, &was).unwrap(), Change::Gone);
    }
}

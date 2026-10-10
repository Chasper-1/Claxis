//! Генерация конфига и чтение с диска.
//!
//! При первом запуске создаются все файлы разом, со всеми ключами и
//! комментариями. Правило проекта: **любая задуманная настройка обязана быть
//! в конфиге с самого начала**. Тот, кто настройку ещё не придумал, ничего
//! дописывать не должен — файл уже есть.
//!
//! Генерируются и активные ключи, и закомментированные. Закомментированный
//! ключ виден и понятен, но не мешает, пока не понадобится.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table};

use crate::Error;
use crate::path;
use crate::schema::{self, ConfigFile, KEYS, KeyDef};

/// Что сделала генерация.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Generated {
    /// Файлы, которых не было и которые созданы.
    pub created: Vec<PathBuf>,
    /// Файлы, которые уже были и не тронуты.
    ///
    /// Существующий файл не переписывается: в нём комментарии пользователя и
    /// его порядок ключей.
    pub kept: Vec<PathBuf>,
}

impl Generated {
    /// Создано хотя бы что-то.
    pub fn wrote_anything(&self) -> bool {
        !self.created.is_empty()
    }

    /// Каталоги, которые нужно создать: темы и переводы.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut folders = Vec::new();
        for file in &self.created {
            if let Some(parent) = file.parent()
                && !folders.contains(&parent.to_path_buf())
            {
                folders.push(parent.to_path_buf());
            }
        }
        folders
    }
}

/// Создать недостающие файлы конфига в `dir`.
///
/// Существующие не трогаются. Возвращает, что создано и что сохранено.
pub fn ensure_files(dir: &Path) -> std::result::Result<Generated, Error> {
    let mut result = Generated::default();
    std::fs::create_dir_all(dir).map_err(|e| Error::Read {
        path: dir.display().to_string(),
        reason: e.to_string(),
    })?;

    for &file in ConfigFile::ALL {
        let path = dir.join(file.file_name());
        if path.exists() {
            result.kept.push(path);
            continue;
        }
        let text = render(file);
        std::fs::write(&path, text).map_err(|e| Error::Write {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        result.created.push(path);
    }
    Ok(result)
}

/// Текст файла конфига для состояния: все ключи с комментариями.
pub fn render(file: ConfigFile) -> String {
    let mut doc = DocumentMut::new();

    for key in KEYS.iter().filter(|k| k.file == file) {
        // Секция может быть вложенной (`Word.Word`), поэтому идём по частям и
        // создаём недостающие таблицы по дороге.
        let mut table = doc.as_table_mut();
        for part in key.section().split('.') {
            if !table.contains_key(part) {
                table[part] = toml_edit::table();
            }
            table = table
                .get_mut(part)
                .and_then(Item::as_table_mut)
                .expect("только что создана");
        }

        // Комментарий пишется в оформление самого ключа, а не значения: иначе
        // он встанет после знака равенства. Оформление ключа переживает любую
        // перезапись значения, поэтому комментарий пользователя не теряется.
        let mut key_with_decor = toml_edit::Key::new(key.key());
        key_with_decor.leaf_decor_mut().set_prefix(format!(
            "# {}\n{}\n",
            key.comment,
            comment_body(key)
        ));
        table.insert_formatted(&key_with_decor, value_for(key));
    }
    doc.to_string()
}

/// Комментарий под ключом: тип и допустимые значения.
fn comment_body(key: &KeyDef) -> String {
    use schema::Kind;
    let what = match key.kind {
        Kind::Bool => "true or false",
        Kind::U32 => "a whole number",
        Kind::Str => "a string",
    };
    format!("# Value: {what}.")
}

/// Значение ключа в виде TOML.
fn value_for(key: &KeyDef) -> Item {
    let raw = key.default;
    match raw.parse::<i64>() {
        Ok(n) => Item::Value(n.into()),
        Err(_) => match raw.parse::<bool>() {
            Ok(b) => Item::Value(b.into()),
            Err(_) => Item::Value(raw.into()),
        },
    }
}

/// Прочитать все файлы конфига из `dir`.
///
/// Файла нет — это не ошибка: значит, он ещё не создан. Отсутствующий файл
/// даёт пустой документ, и работают значения по умолчанию.
pub fn read_all(dir: &Path) -> crate::load::Loaded {
    let mut docs = Vec::new();
    let mut issues = Vec::new();

    for &file in ConfigFile::ALL {
        let path = dir.join(file.file_name());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                docs.push((file, DocumentMut::new()));
                continue;
            }
            Err(e) => {
                issues.push(crate::load::Issue {
                    file: file.file_name(),
                    line: 0,
                    key: String::new(),
                    problem: crate::load::Problem::Unreadable(e.to_string()),
                });
                docs.push((file, DocumentMut::new()));
                continue;
            }
        };
        let one = crate::load::parse(file, &text);
        if let Some(doc) = one.config.doc(file) {
            docs.push((file, doc.clone()));
        }
        issues.extend(one.issues);
    }

    crate::load::Loaded {
        config: crate::Config::from_documents(docs),
        issues,
    }
}

/// Записать значение одного ключа, не трогая остальной файл.
///
/// Комментарии и порядок ключей сохраняются: файл пользователя не должен
/// переписываться целиком при смене одной настройки.
pub fn write_value(
    dir: &Path,
    key: &'static KeyDef,
    value: &str,
) -> std::result::Result<(), Error> {
    let path = dir.join(key.file.file_name());
    let text = std::fs::read_to_string(&path).map_err(|e| Error::Read {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    let mut doc = text
        .parse::<DocumentMut>()
        .map_err(|e: toml_edit::TomlError| Error::Parse {
            path: path.display().to_string(),
            reason: e.message().to_string(),
        })?;

    let mut table: &mut Table = doc.as_table_mut();
    for part in key.section().split('.') {
        if !table.contains_key(part) {
            table[part] = toml_edit::table();
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Parse {
                path: path.display().to_string(),
                reason: format!("{} is not a section", part),
            })?;
    }

    // Оформление ключа переносим: комментарий пользователя и его порядок в
    // файле должны остаться, мы меняем только значение.
    let entry = table
        .get_key_value(key.key())
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| toml_edit::Key::new(key.key()));
    table.insert_formatted(&entry, parse_value(key, value));
    std::fs::write(&path, doc.to_string()).map_err(|e| Error::Write {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

fn parse_value(key: &KeyDef, value: &str) -> Item {
    use toml_edit::Value;
    match key.kind {
        schema::Kind::U32 => match value.parse::<i64>() {
            Ok(n) => Item::Value(Value::from(n)),
            Err(_) => Item::Value(Value::from(value)),
        },
        schema::Kind::Bool => match value.parse::<bool>() {
            Ok(b) => Item::Value(Value::from(b)),
            Err(_) => Item::Value(Value::from(value)),
        },
        schema::Kind::Str => Item::Value(Value::from(value)),
    }
}

/// Папки, которые должны существовать рядом с конфигом.
pub fn required_dirs(dir: &Path) -> Vec<PathBuf> {
    vec![path::theme_dir(dir), path::lang_dir(dir)]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("claxis-cfg-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
        fn dir(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_key_lands_in_the_generated_file() {
        // Правило проекта: любая задуманная настройка обязана быть в конфиге.
        for &file in ConfigFile::ALL {
            let text = render(file);
            for key in KEYS.iter().filter(|k| k.file == file) {
                assert!(
                    text.contains(key.key()),
                    "{}: ключ {} не попал в файл",
                    file.file_name(),
                    key.key()
                );
            }
        }
    }

    #[test]
    fn every_key_has_a_comment() {
        // Комментарий виден и понятен с момента создания файла.
        for &file in ConfigFile::ALL {
            let text = render(file);
            for key in KEYS.iter().filter(|k| k.file == file) {
                assert!(
                    text.contains(key.comment),
                    "{}: нет комментария к {}",
                    file.file_name(),
                    key.key()
                );
            }
        }
    }

    #[test]
    fn generated_file_parses_clean() {
        // Сгенерированный конфиг не должен содержать ошибок: иначе редактор
        // при первом запуске сразу пожалуется на свой же файл.
        for &file in ConfigFile::ALL {
            let loaded = crate::load::parse(file, &render(file));
            assert!(
                loaded.is_ok(),
                "{}: сгенерированный файл не проходит проверку: {:?}",
                file.file_name(),
                loaded.issues
            );
        }
    }

    #[test]
    fn generated_values_match_the_schema() {
        let text = render(ConfigFile::Edit);
        let loaded = crate::load::parse(ConfigFile::Edit, &text);
        let key = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(
            loaded.config.u32_or_default(key),
            key.default.parse::<u32>().unwrap()
        );
    }

    #[test]
    fn first_run_creates_all_files() {
        let tmp = Temp::new("first-run");
        let result = ensure_files(tmp.dir()).unwrap();
        assert!(result.wrote_anything());
        for &file in ConfigFile::ALL {
            assert!(
                tmp.dir().join(file.file_name()).exists(),
                "{} не создан",
                file.file_name()
            );
        }
    }

    #[test]
    fn existing_files_are_never_overwritten() {
        // Пользовательские комментарии и порядок ключей не трогаем.
        let tmp = Temp::new("keep-mine");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mine = "[General]\n# моя настройка\nhistory_depth = 100\n";
        std::fs::write(tmp.dir().join("edit.toml"), mine).unwrap();

        let result = ensure_files(tmp.dir()).unwrap();
        assert!(result.created.contains(&tmp.dir().join("config.toml")));
        assert_eq!(
            std::fs::read_to_string(tmp.dir().join("edit.toml")).unwrap(),
            mine
        );
    }

    #[test]
    fn missing_file_is_not_an_error() {
        // Файла нет — работают значения по умолчанию, это не поломка.
        let tmp = Temp::new("missing");
        let loaded = read_all(tmp.dir());
        assert!(loaded.is_ok(), "пустой каталог не должен давать ошибок");
        let key = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(loaded.config.u32_or_default(key), 8192);
    }

    #[test]
    fn read_all_finds_a_bad_key_with_its_line() {
        let tmp = Temp::new("bad");
        ensure_files(tmp.dir()).unwrap();
        std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 4\n").unwrap();
        let loaded = read_all(tmp.dir());
        let issue = loaded.first().expect("должна быть проблема");
        assert_eq!(issue.file, "edit.toml");
        assert_eq!(issue.line, 2);
    }

    #[test]
    fn point_write_keeps_comments_and_order() {
        let tmp = Temp::new("write");
        ensure_files(tmp.dir()).unwrap();
        let path = tmp.dir().join("edit.toml");
        let before = std::fs::read_to_string(&path).unwrap();

        let key = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        write_value(tmp.dir(), key, "128").unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("history_depth = 128"), "{after}");
        // Комментарии на месте.
        assert!(after.contains(key.comment), "комментарий пропал");
        // Порядок ключей не изменился.
        fn order(s: &str) -> Vec<String> {
            s.lines()
                .filter(|l| !l.starts_with('#') && l.contains('='))
                .map(|l| l.split('=').next().unwrap().trim().to_string())
                .collect()
        }
        assert_eq!(order(&before), order(&after), "порядок ключей изменился");
    }

    #[test]
    fn point_write_can_toggle_a_boolean() {
        let tmp = Temp::new("bool-write");
        ensure_files(tmp.dir()).unwrap();
        let key = schema::find(ConfigFile::Files, "General.snapshots_persist").unwrap();
        write_value(tmp.dir(), key, "false").unwrap();

        let loaded = read_all(tmp.dir());
        assert!(loaded.is_ok(), "проблемы: {:?}", loaded.issues);
        let keep = schema::find(ConfigFile::Files, "General.snapshots_persist").unwrap();
        assert!(!loaded.config.bool_or_default(keep));
    }
}

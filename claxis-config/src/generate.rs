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
pub fn ensure_files(
    dir: &Path,
    catalog: &claxis_i18n::Catalog,
) -> std::result::Result<Generated, Error> {
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
        let text = render(file, catalog);
        std::fs::write(&path, text).map_err(|e| Error::Write {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        result.created.push(path);
    }
    Ok(result)
}

/// Текст файла конфига для состояния: все ключи с комментариями.
///
/// Файл собирается текстом, а не через `toml_edit`: так можно вывести
/// неподключённый ключ закомментированным, не мучаясь с оформлением. Разбор
/// всё равно проверяет результат тестом — сгенерированный файл обязан быть
/// корректным.
pub fn render(file: ConfigFile, catalog: &claxis_i18n::Catalog) -> String {
    let mut out = String::new();
    let mut section = "";

    for key in KEYS.iter().filter(|k| k.file == file) {
        if key.section() != section {
            section = key.section();
            out.push_str(&format!("\n[{section}]\n\n"));
        }
        // Комментарий — тоже текст для пользователя, значит тоже переводится.
        // Ключом служит английская фраза: каталог ищет её как есть.
        out.push_str(&format!("# {}\n", tr(catalog, key.comment)));
        out.push_str(&format!("{}\n", comment_body(key, catalog)));
        // Строка обязана быть в кавычках: раскомментированный ключ должен
        // остаться корректным, иначе файл перестанет читаться.
        let value = value_text(key);
        if key.active {
            out.push_str(&format!("{} = {value}\n\n", key.key()));
        } else {
            // Ключ есть и понятен, но значения ему не назначено, поэтому он
            // не действует. Причина в самом ключе, а не в коде: пользователю
            // не нужно знать, что внутри.
            out.push_str(&format!("# {}\n", tr(catalog, NOT_SET)));
            out.push_str(&format!("#{} = {value}\n\n", key.key()));
        }
    }
    out
}

/// Простое: ключ перевода — английская фраза, как её писал переводчик.
pub const NOT_SET: &str = "Not set: no value is assigned to this key.";

/// Перевод английской фразы. Без перевода остаётся английский.
fn tr<'a>(catalog: &'a claxis_i18n::Catalog, text: &'a str) -> &'a str {
    catalog.get(text).unwrap_or(text)
}

/// Значение ключа так, как оно записывается в TOML.
fn value_text(key: &KeyDef) -> String {
    match key.kind {
        schema::Kind::Str => format!("\"{}\"", key.default),
        _ => key.default.to_string(),
    }
}

/// Комментарий под ключом: тип и допустимые значения.
fn comment_body(key: &KeyDef, catalog: &claxis_i18n::Catalog) -> String {
    // Вся фраза целиком идёт через каталог: переводчик сам решает, где у него
    // стоит слово «значение», а где название типа.
    let phrase = match key.kind {
        schema::Kind::Bool => "Value: true or false.",
        schema::Kind::U32 => "Value: a whole number.",
        schema::Kind::Str => "Value: a string.",
    };
    format!("# {}", tr(catalog, phrase))
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

    /// Пустой каталог: без перевода тексты остаются английскими.
    fn plain() -> claxis_i18n::Catalog {
        claxis_i18n::Catalog::new()
    }

    /// Каталог с переводом, чтобы проверить, что перевод попадает в файл.
    fn russian() -> claxis_i18n::Catalog {
        claxis_i18n::Catalog::from_entries(
            [
                (
                    "Not set: no value is assigned to this key.",
                    "Не задано: ключу не назначено значение.",
                ),
                ("Value: a whole number.", "Значение: целое число."),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string())),
        )
    }

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
            let text = render(file, &plain());
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
            let text = render(file, &plain());
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
            let loaded = crate::load::parse(file, &render(file, &plain()));
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
        let text = render(ConfigFile::Edit, &plain());
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
        let result = ensure_files(tmp.dir(), &plain()).unwrap();
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
    fn unused_keys_are_written_commented_out() {
        // Неподключённый ключ есть и понятен, но выключен: пользователь не
        // должен править настройку, которая ничего не делает, не зная об этом.
        let text = render(ConfigFile::Global, &plain());
        assert!(text.contains("#theme = \"default\""), "{text}");
        assert!(!text.contains("\ntheme ="), "выключенный ключ не активен");
    }

    #[test]
    fn used_keys_are_written_active() {
        let text = render(ConfigFile::Edit, &plain());
        assert!(text.contains("history_depth = 8192"), "{text}");
        assert!(
            !text.contains("#history_depth"),
            "подключённый ключ не должен быть закомментирован"
        );
    }

    #[test]
    fn commented_key_becomes_valid_when_uncommented() {
        // Раскомментированный ключ должен остаться разбираемым: иначе файл
        // сломается ровно тогда, когда пользователь решит им заняться.
        let text = render(ConfigFile::Global, &plain());
        let uncommented: String = text
            .lines()
            .map(|l| {
                l.strip_prefix('#')
                    .filter(|r| r.contains(" = "))
                    .unwrap_or(l)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let loaded = crate::load::parse(ConfigFile::Global, &uncommented);
        assert!(
            loaded.is_ok(),
            "раскомментированный файл не разбирается: {:?}",
            loaded.issues
        );
        for key in KEYS.iter().filter(|k| k.file == ConfigFile::Global) {
            assert!(
                uncommented.contains(key.key()),
                "ключ {} потерялся",
                key.path
            );
        }
    }

    #[test]
    fn active_flag_matches_the_code() {
        // Правдивость флага: подключено ровно то, что код читает. Список
        // читается руками, но тест заставит его обновить, когда читать
        // станет больше.
        const READ: &[&str] = &[
            "General.history_depth",
            "General.snapshots_keep",
            "General.snapshots_persist",
        ];
        for key in KEYS {
            let should = READ.contains(&key.path);
            assert_eq!(
                key.active, should,
                "{}: код читает, а флаг говорит {:?}. Список READ в тесте обновить.",
                key.path, key.active
            );
        }
    }

    #[test]
    fn comments_are_translated() {
        // Комментарий в конфиге — такой же текст для пользователя, как и всё
        // остальное, значит он переводится.
        let text = render(ConfigFile::Global, &russian());
        assert!(
            text.contains("Не задано: ключу не назначено значение."),
            "перевод предупреждения не попал в файл: {text}"
        );
        assert!(
            text.contains("целое число"),
            "перевод типа не попал: {text}"
        );
    }

    #[test]
    fn every_generated_comment_goes_through_the_catalog() {
        // Ни одна строка комментариев не должна быть зашита в обход каталога.
        // Проверяем косвенно: без перевода все фразы английские, и каждая
        // встречается как ключ каталога.
        let mut text = String::new();
        for &file in ConfigFile::ALL {
            text.push_str(&render(file, &plain()));
        }
        for phrase in [
            NOT_SET,
            "Value: a whole number.",
            "Value: true or false.",
            "Value: a string.",
        ] {
            assert!(
                text.contains(phrase),
                "фраза {phrase} не проходит через каталог"
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

        let result = ensure_files(tmp.dir(), &plain()).unwrap();
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
        ensure_files(tmp.dir(), &plain()).unwrap();
        std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 4\n").unwrap();
        let loaded = read_all(tmp.dir());
        let issue = loaded.first().expect("должна быть проблема");
        assert_eq!(issue.file, "edit.toml");
        assert_eq!(issue.line, 2);
    }

    #[test]
    fn point_write_keeps_comments_and_order() {
        let tmp = Temp::new("write");
        ensure_files(tmp.dir(), &plain()).unwrap();
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
        ensure_files(tmp.dir(), &plain()).unwrap();
        let key = schema::find(ConfigFile::Files, "General.snapshots_persist").unwrap();
        write_value(tmp.dir(), key, "false").unwrap();

        let loaded = read_all(tmp.dir());
        assert!(loaded.is_ok(), "проблемы: {:?}", loaded.issues);
        let keep = schema::find(ConfigFile::Files, "General.snapshots_persist").unwrap();
        assert!(!loaded.config.bool_or_default(keep));
    }
}

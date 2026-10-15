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
    // Секции, которые уже выведены: секция с встроенной таблицей может иметь
    // и обычные ключи, и тогда она печатается дважды, а TOML такой файл не
    // разбирает.
    let mut shown: Vec<&str> = Vec::new();

    for key in KEYS.iter().filter(|k| k.file == file) {
        if key.section() != section {
            section = key.section();
            shown.push(section);
            out.push_str(&format!("\n[{section}]\n\n"));
            out.push_str(&table_body(key.file, section, catalog));
        }
        // Комментарий — тоже текст для пользователя, значит тоже переводится.
        // Ключом служит английская фраза: каталог ищет её как есть.
        //
        // Комментарий стоит на той же строке, что ключ: так видно, к чему он
        // относится, и файл не раздувается вдвое. Одна строка — одно
        // предложение плюс допустимые значения, больше ничего.
        let value = value_text(key);
        let comment = key_comment(key, catalog);
        if key.active {
            out.push_str(&format!("{} = {value}  # {comment}\n\n", key.key()));
        } else {
            // Ключ есть и понятен, но значения ему не назначено, поэтому он
            // не действует. Причина в самом ключе, а не в коде: пользователю
            // не нужно знать, что внутри.
            out.push_str(&format!("#{} = {value}  # {comment}\n\n", key.key()));
        }
    }

    // Секция может состоять только из встроенной таблицы: обычных ключей в
    // ней нет, а сама настройка обязана попасть в файл, иначе пользователю
    // нечем её настроить.
    for table in schema::TABLES.iter().filter(|t| t.file == file) {
        let table_section = section_prefix(table);
        if shown.contains(&table_section) {
            continue;
        }
        out.push_str(&format!("\n[{table_section}]\n\n"));
        out.push_str(&table_body(file, table_section, catalog));
        shown.push(table_section);
    }
    out
}

/// Тело табличной настройки, если она есть в этой секции.
///
/// Значение само является таблицей, а имена её ключей — данные, которые задаёт
/// пользователь: сколько клавиш настроить, решает он.
fn table_body(file: schema::ConfigFile, section: &str, catalog: &claxis_i18n::Catalog) -> String {
    let Some(table) = schema::TABLES
        .iter()
        .find(|t| t.file == file && section_prefix(t) == section)
    else {
        return String::new();
    };
    let pairs: Vec<String> = table
        .default
        .iter()
        // Строковое значение пишется в кавычках, иначе это будет не строка,
        // а имя: `percent = {used}` разбор отвергнет.
        .map(|(k, v)| match table.kind {
            schema::Kind::Str => format!("{k} = \"{v}\""),
            _ => format!("{k} = {v}"),
        })
        .collect();
    // Таблица пишется по строке на пару: в одну строку её не прочитать, а
    // разбирать мы будем сворачивая — см. `load::fold_inline_tables`.
    format!(
        "{} = {{\n  {}\n}}  # {}\n\n",
        table_name(table),
        pairs.join(",\n  "),
        tr(catalog, table.comment),
    )
}

/// Где в строке закрывающая скобка встроенной таблицы.
///
/// Обычный `find('}')` не годится: в значении вида `{used}` скобка стоит
/// внутри строки и закрыла бы таблицу на полуслове.
fn closing_brace(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '}' => return Some(i),
            _ => {}
        }
    }
    None
}

/// Разделить пары встроенной таблицы по запятым снаружи строк.
///
/// По обычному `split(", ")` пара вида `text = "a, b"` распалась бы на две, и
/// в конфиге появилось бы значение, которого никто не писал.
fn split_pairs(pairs: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in pairs.chars() {
        if let Some(q) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                current.push(c);
            }
            ',' => {
                if current.trim().is_empty() {
                    // Две запятые подряд: пустая пара, её пропускаем.
                    current.clear();
                } else {
                    out.push(current.trim().to_string());
                    current.clear();
                }
            }
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// Развернуть встроенные таблицы схемы по строкам.
///
/// Только те, что принадлежат схеме: чужое оформление пользователя не
/// трогаем, переписываем только то, что сами пишем. Пары берутся из самой
/// строки, поэтому разворачивается и таблица, которую пользователь поправил.
pub fn expand_schema_tables(text: &str) -> String {
    let names: Vec<&str> = schema::TABLES.iter().map(table_name).collect();
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        out.push_str(&expand_line(line, &names));
        out.push('\n');
    }
    out
}

/// Развернуть одну строку, если это таблица схемы.
fn expand_line(line: &str, names: &[&str]) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let trimmed = &line[indent_len..];

    let Some(name) = names
        .iter()
        .find(|n| trimmed.starts_with(&format!("{n} = {{")))
    else {
        return line.to_string();
    };
    // Минус четыре: пробел, `=`, пробел и сама `{`. Дальше идут пары.
    let rest = &trimmed[name.len() + 4..];
    let Some(close) = closing_brace(rest) else {
        return line.to_string();
    };
    let pairs = &rest[..close];
    let tail = rest[close + 1..].trim_start();

    // Пустая таблица в одну строку остаётся в одну строку: переносить нечего.
    if pairs.trim().is_empty() {
        return line.to_string();
    }
    let indent_pair = format!("{indent}  ");
    let body = split_pairs(pairs)
        .iter()
        .map(|p| format!("{indent_pair}{}", p.trim()))
        .collect::<Vec<_>>()
        .join(",\n");

    let mut out = format!("{indent}{name} = {{\n{body}\n{indent}}}");
    if !tail.is_empty() {
        out.push(' ');
        out.push_str(tail);
    }
    out
}

/// Секция, в которой живёт табличная настройка: `General.hold` → `General`.
fn section_prefix(table: &schema::TableDef) -> &str {
    table.path.rsplit_once('.').map_or("", |(s, _)| s)
}

/// Имя настройки, значение которой таблица: `General.hold` → `hold`.
fn table_name(table: &schema::TableDef) -> &str {
    table.path.rsplit('.').next().unwrap_or(table.path)
}

/// Ключ есть в конфиге, но не применяется.
pub const NOT_USED: &str = "Not in use.";

/// Перевод английской фразы. Без перевода остаётся английский.
fn tr<'a>(catalog: &'a claxis_i18n::Catalog, text: &'a str) -> &'a str {
    catalog.get(text).unwrap_or(text)
}

/// Допустимые значения для каталога.
///
/// Только диапазон: человек проверит — получит ошибку, которая подтвердит.
fn range_hint(kind: schema::Kind) -> &'static str {
    match kind {
        schema::Kind::Bool => "true or false",
        schema::Kind::U16 => "0 to 65535",
        schema::Kind::U8 => "0 to 255",
        schema::Kind::Str | schema::Kind::Array | schema::Kind::Path => "",
        // У целого со знаком и у дробного верхней границы нет: она не
        // помещается в подсказку и всё равно не нужна человеку.
        schema::Kind::Int | schema::Kind::Float => "",
    }
}

/// Значение ключа так, как оно записывается в TOML.
fn value_text(key: &KeyDef) -> String {
    match key.kind {
        schema::Kind::Str | schema::Kind::Path => format!("\"{}\"", key.default),
        // Список пишется в квадратных скобках, иначе это строка, а не список,
        // и при раскомментировании файл перестанет разбираться.
        schema::Kind::Array => format!("[{}]", key.default),
        _ => key.default.to_string(),
    }
}

/// Комментарий на строке с ключом: что значит и какие значения принимает.
///
/// Одно предложение плюс диапазон. Всё целиком идёт через каталог:
/// переводчик сам решает, где у него стоит слово «значение».
fn key_comment(key: &KeyDef, catalog: &claxis_i18n::Catalog) -> String {
    let mut text = tr(catalog, key.comment).to_string();
    let range = range_hint(key.kind);
    if !range.is_empty() {
        text.push_str(&format!(" {}", tr(catalog, range)));
    }
    if !key.active {
        text.push_str(&format!(". {}", tr(catalog, NOT_USED)));
    }
    text
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
    let mut doc = open_for_edit(&path)?;
    let table = section_mut(&mut doc, &path, key.section())?;

    // Оформление ключа переносим: комментарий пользователя и его порядок в
    // файле должны остаться, мы меняем только значение.
    let entry = keep_formatting(table, key.key());
    // Комментарий стоит на строке с ключом, а вставка значения его стирает.
    // Забираем суффикс до вставки и возвращаем после: меняется только число.
    let suffix = take_suffix(table, key.key());
    table.insert_formatted(&entry, parse_value(key, value));
    if let Some(suffix) = suffix
        && let Some(v) = table.get_mut(key.key()).and_then(Item::as_value_mut)
    {
        v.decor_mut().set_suffix(suffix);
    }
    save(&path, &doc)
}

/// Прочитать файл конфига на правку.
///
/// Многострочные таблицы схемы перед разбором сворачиваются в строку: так их
/// удобно разбирать. Обратно они расправляются при записи.
fn open_for_edit(path: &Path) -> std::result::Result<DocumentMut, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Read {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    crate::load::fold_inline_tables(&text)
        .parse::<DocumentMut>()
        .map_err(|e: toml_edit::TomlError| Error::Parse {
            path: path.display().to_string(),
            reason: e.message().to_string(),
        })
}

/// Записать документ обратно, расправив свёрнутые таблицы схемы.
fn save(path: &Path, doc: &DocumentMut) -> std::result::Result<(), Error> {
    let written = expand_schema_tables(&doc.to_string());
    std::fs::write(path, written).map_err(|e| Error::Write {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

/// Спуститься по пути секции, создавая недостающие таблицы на ходу.
fn section_mut<'a>(
    doc: &'a mut DocumentMut,
    path: &Path,
    section: &str,
) -> std::result::Result<&'a mut Table, Error> {
    let mut table = doc.as_table_mut();
    for part in section.split('.') {
        if !table.contains_key(part) {
            table[part] = toml_edit::table();
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::NotASection {
                path: path.display().to_string(),
                part: part.to_string(),
            })?;
    }
    Ok(table)
}

/// Ключ в его нынешнем виде: с комментарием и порядком, как у человека.
fn keep_formatting(table: &Table, name: &str) -> toml_edit::Key {
    table
        .get_key_value(name)
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| toml_edit::Key::new(name))
}

/// Комментарий на строке ключа: без него правка стёрла бы пояснение.
fn take_suffix(table: &Table, name: &str) -> Option<String> {
    table
        .get(name)
        .and_then(Item::as_value)
        .and_then(|v| v.decor().suffix())
        .map(|s| s.as_str().unwrap_or_default().to_string())
}

/// Записать пары табличной настройки, не трогая остальной файл.
///
/// Таблица читается человеком по строкам, значит и записывается по строкам,
/// а комментарий ключа остаётся на месте.
pub fn write_table_value(
    dir: &Path,
    def: &'static schema::TableDef,
    pairs: &[(&str, &str)],
) -> std::result::Result<(), Error> {
    let path = dir.join(def.file.file_name());
    let mut doc = open_for_edit(&path)?;
    let section = section_prefix(def);
    let name = table_name(def);
    let table = section_mut(&mut doc, &path, section)?;

    // Оформление ключа переносим: комментарий пользователя должен остаться.
    let entry = keep_formatting(table, name);
    let suffix = take_suffix(table, name);

    let mut inline = toml_edit::InlineTable::new();
    // Пары схемы — основание: переданные переопределяют их, а забытые
    // остаются, иначе таблица теряла бы половину настроек при правке одной.
    for (key, value) in def.default {
        inline.insert(*key, value_for_table(def, value));
    }
    for (key, value) in pairs {
        inline.insert(*key, value_for_table(def, value));
    }
    table.insert_formatted(&entry, Item::Value(toml_edit::Value::InlineTable(inline)));
    if let Some(suffix) = suffix
        && let Some(v) = table.get_mut(name).and_then(Item::as_value_mut)
    {
        v.decor_mut().set_suffix(suffix);
    }
    save(&path, &doc)
}

/// Значение пары таблицы: строка получает кавычки, остальное пишется как есть.
fn value_for_table(def: &schema::TableDef, value: &str) -> toml_edit::Value {
    match def.kind {
        schema::Kind::Str | schema::Kind::Path => toml_edit::Value::from(value),
        schema::Kind::Bool => value
            .parse::<bool>()
            .map_or_else(|_| toml_edit::Value::from(value), toml_edit::Value::from),
        schema::Kind::U8 | schema::Kind::U16 | schema::Kind::Int => value
            .parse::<i64>()
            .map_or_else(|_| toml_edit::Value::from(value), toml_edit::Value::from),
        schema::Kind::Float => value
            .parse::<f64>()
            .map_or_else(|_| toml_edit::Value::from(value), toml_edit::Value::from),
        schema::Kind::Array => {
            let mut array = toml_edit::Array::new();
            for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                array.push(part.trim_matches('"'));
            }
            toml_edit::Value::Array(array)
        }
    }
}

/// Значение ключа в виде, который понимает TOML.
///
/// Открыто наружу: тем же правилом проверяется тип при разборе, иначе ключ
/// можно было бы записать не так, как потом разберётся.
pub fn parse_value(key: &KeyDef, value: &str) -> Item {
    use toml_edit::Value;
    match key.kind {
        schema::Kind::U16 | schema::Kind::U8 => match value.parse::<i64>() {
            Ok(n) => Item::Value(Value::from(n)),
            Err(_) => Item::Value(Value::from(value)),
        },
        schema::Kind::Bool => match value.parse::<bool>() {
            Ok(b) => Item::Value(Value::from(b)),
            Err(_) => Item::Value(Value::from(value)),
        },
        schema::Kind::Str | schema::Kind::Path => Item::Value(Value::from(value)),
        // Список приходит текстом через запятую и собирается в массив: так
        // его можно и показать в конфиге, и положить обратно без потерь.
        schema::Kind::Array => {
            // Схема хранит список текстом через запятую и без внешних скобок,
            // а в TOML он снова список — со скобками и без кавычек внутри.
            let mut array = toml_edit::Array::new();
            for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                array.push(part.trim_matches('"'));
            }
            Item::Value(Value::Array(array))
        }
        schema::Kind::Int => match value.parse::<i64>() {
            Ok(n) => Item::Value(Value::from(n)),
            Err(_) => Item::Value(Value::from(value)),
        },
        schema::Kind::Float => match value.parse::<f64>() {
            Ok(f) => Item::Value(Value::from(f)),
            Err(_) => match value.parse::<i64>() {
                Ok(n) => Item::Value(Value::from(n)),
                Err(_) => Item::Value(Value::from(value)),
            },
        },
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
                    "Keys with two meanings, and how long a press counts as held, in milliseconds.",
                    "Клавиши со значением, и окно удержания для каждой.",
                ),
                ("Not in use.", "Не используется."),
                (
                    "Edits kept in memory before taking a snapshot.",
                    "Правок в памяти до снапшота.",
                ),
                ("0 to 65535", "от 0 до 65535"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string())),
        )
    }

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("claxis-cfg-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            // Каталога может не быть, а тест пишет в него файл: создаём
            // сразу, иначе падение будет не про то, что проверяем.
            std::fs::create_dir_all(&p).expect("каталог создаётся");
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
            loaded.config.u16_or_default(key),
            key.default.parse::<u16>().unwrap()
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
    fn active_flag_matches_the_code() {
        // Правдивость флага: подключено ровно то, что код читает. Список
        // читается руками, но тест заставит его обновить, когда читать
        // станет больше.
        const READ: &[&str] = &[
            "General.language",
            "General.theme",
            "General.history_depth",
            "General.snapshots_keep",
            "General.snapshots_persist",
            "General.snapshots_interval",
            "Savescreen.include",
            "Savescreen.mode",
            "Savescreen.Statistic.input_char",
            "Savescreen.Statistic.delete_char",
            "Savescreen.Statistic.history_usage",
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
    fn used_theme_and_language_are_written_active() {
        // Язык и тема работают, значит в свежем конфиге они не закомментированы:
        // иначе пришлось бы раскомментировать их руками перед первым же
        // использованием.
        let text = render(ConfigFile::Global, &plain());
        assert!(text.contains("theme = \"default\""), "{text}");
        assert!(text.contains("language = \"en\""), "{text}");
        assert!(
            !text.contains("#theme ="),
            "тема не закомментирована: {text}"
        );
        assert!(
            !text.contains("#language ="),
            "язык не закомментирован: {text}"
        );
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
    fn every_kind_survives_a_round_trip() {
        // Каждый вид значения обязан и записаться в файл, и пройти проверку
        // своего типа. Не проверив это, сломанный вид всплыл бы на живом
        // конфиге.
        use crate::schema::{ConfigFile as File, KeyDef, Kind};
        const CASES: &[(Kind, &str, &str)] = &[
            (Kind::Bool, "true", "flag"),
            (Kind::U8, "200", "small"),
            (Kind::U16, "8192", "depth"),
            (Kind::Int, "-5", "offset"),
            (Kind::Float, "0.25", "ratio"),
            (Kind::Str, "hello", "name"),
            (Kind::Path, "/tmp/x", "file"),
            (Kind::Array, "\"a\", \"b\"", "list"),
        ];
        for (kind, value, name) in CASES {
            let key = KeyDef {
                file: File::Global,
                path: "General.probe",
                default: value,
                kind: *kind,
                comment: "probe",
                active: true,
            };
            assert!(
                crate::load::value_matches(&key, value),
                "{name}: значение {value} не прошло проверку своего типа"
            );

            // И обратно: значение должно записаться в файл и разобраться.
            let line = format!("probe = {}", value_text(&key));
            let mut doc = toml_edit::DocumentMut::new();
            doc["General"]["probe"] = parse_value(&key, value);
            let written = doc.to_string();
            assert!(
                written.contains(line.trim_end()),
                "{name}: записалось не то, что показано: {written}"
            );
            assert!(
                written.trim().parse::<toml_edit::DocumentMut>().is_ok(),
                "{name}: записанный файл не разбирается: {written}"
            );
        }
    }

    #[test]
    fn a_list_is_not_a_string_and_its_inners_are_checked() {
        // Список строк не должен проходить проверку, если внутри не строки,
        // иначе опечатка внутри списка дойдёт до кода как есть.
        use crate::schema::{ConfigFile as File, KeyDef, Kind};
        let key = KeyDef {
            file: File::Global,
            path: "General.probe",
            default: "\"a\"",
            kind: Kind::Array,
            comment: "probe",
            active: true,
        };
        assert!(crate::load::value_matches(&key, "\"a\", \"b\""));
        assert!(
            crate::load::value_matches(&key, ""),
            "пустой список тоже список"
        );
        assert!(!crate::load::value_matches(&key, "a"), "строка не список");
    }

    #[test]
    fn a_wrong_type_is_named_with_its_kind() {
        // Ошибка должна говорить, чего ждали: «список», а не «строка».
        use crate::schema::Kind;
        let def = crate::schema::find(ConfigFile::Global, "General.language").unwrap();
        assert_eq!(def.kind, Kind::Str);
        assert_eq!(Kind::Array.expected(), "a list of strings");
        assert_eq!(Kind::Path.expected(), "a path");
        assert_eq!(Kind::Int.expected(), "a whole number");
        assert_eq!(Kind::Float.expected(), "a number with a fractional part");
    }

    #[test]
    fn a_multiline_table_is_read_the_way_a_person_writes_it() {
        // Главное ради чего всё затевалось: таблицу, разбитую по строкам,
        // должно быть можно написать руками. TOML 1.0 её не берёт, поэтому мы
        // сворачиваем таблицу перед разбором.
        let text = "[Savescreen.Setting]\nhistory_usage = {\n  percent = \"{used}\",\n  number = \"{remaining}\"\n}\n";
        let loaded = crate::load::parse(ConfigFile::Global, text);
        assert!(
            loaded.issues.is_empty(),
            "многострочная таблица не разобралась: {:?}",
            loaded.issues
        );
        let table =
            crate::schema::find_table(ConfigFile::Global, "Savescreen.Setting.history_usage")
                .unwrap();
        let got = loaded.config.table(table);
        // Таблица отдаёт значения как есть в TOML, вместе с кавычками:
        // подпись нужна сама по себе, без них.
        let plain: Vec<(String, String)> = got
            .into_iter()
            .map(|(k, v)| (k, v.trim_matches('"').to_string()))
            .collect();
        assert_eq!(
            plain,
            vec![
                ("percent".to_string(), "{used}".to_string()),
                ("number".to_string(), "{remaining}".to_string()),
            ],
            "подписи прочитались не те"
        );
    }

    #[test]
    fn folding_leaves_everything_else_where_it_was() {
        // Сворачивание трогает только таблицы: переносы строк снаружи, в
        // именах и в комментариях обязаны остаться на своих местах, иначе
        // номер строки в ошибке соврёт.
        let text = "# шапка\n[General]\nname = \"a { b } c\"  # скобки в строке\nhold = { alt = 200, ctrl = 200 }\nпосле = 1\n";
        let folded = crate::load::fold_inline_tables(text);
        assert_eq!(
            folded.lines().count(),
            text.lines().count(),
            "строк не потерялось"
        );
        assert!(
            folded.contains("name = \"a { b } c\""),
            "скобки в строке не тронуты"
        );
        assert!(
            folded.contains("hold = { alt = 200, ctrl = 200 }"),
            "таблица не поехала"
        );
        assert!(folded.contains("после = 1"), "хвост не тронут");
    }

    #[test]
    fn a_key_where_a_section_should_be_is_reported() {
        // Путь к секции занят значением: идти вглубь нельзя, и об этом сказано
        // внятным видом ошибки, а не готовым текстом внутри `Parse`.
        let tmp = Temp::new("not-a-section");
        std::fs::write(tmp.dir().join("config.toml"), "Savescreen = \"строка\"\n").unwrap();
        let table =
            crate::schema::find_table(ConfigFile::Global, "Savescreen.Setting.history_usage")
                .unwrap();
        let err = write_table_value(tmp.dir(), table, &[("percent", "{percent}")]).unwrap_err();
        assert!(
            matches!(err, Error::NotASection { .. }),
            "получено: {err:?}"
        );
        assert!(
            err.text().contains("Savescreen"),
            "{text}",
            text = err.text()
        );
    }

    #[test]
    fn a_multiline_table_survives_a_value_change() {
        // Точечная запись не должна возвращать файл к однострочному виду:
        // человек писал по строкам, значит и после правки должно быть так.
        let tmp = Temp::new("multiline-write");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[Savescreen.Setting]\nhistory_usage = {\n  percent = \"{used}\",\n  number = \"{remaining}\"\n}\n",
        )
        .unwrap();
        let table =
            crate::schema::find_table(ConfigFile::Global, "Savescreen.Setting.history_usage")
                .unwrap();
        write_table_value(tmp.dir(), table, &[("percent", "{percent}")]).unwrap();

        let after = std::fs::read_to_string(tmp.dir().join("config.toml")).unwrap();
        assert!(
            after.contains("percent = \"{percent}\""),
            "новое значение не записалось: {after}"
        );
        assert!(
            after.contains("{\n"),
            "таблица схлопнулась в строку: {after}"
        );
        assert!(
            after.contains("number = \"{remaining}\""),
            "остальное потерялось"
        );

        // И файл после правки всё ещё разбирается.
        let loaded = crate::load::parse(ConfigFile::Global, &after);
        assert!(
            loaded.issues.is_empty(),
            "файл после правки не разобрался:\n{after}\nсвёрнуто:\n{}",
            crate::load::fold_inline_tables(&after)
        );
    }

    #[test]
    fn a_comment_inside_a_multiline_table_stays_visible() {
        // Комментарий не может остаться в скобках — TOML его там не допускает,
        // но и пропасть он не должен: человек его написал и должен его видеть.
        let text = "[Savescreen.Setting]\nhistory_usage = {\n  percent = \"{used}\",  # как показывать проценты\n  number = \"{remaining}\"  # как показывать числа\n}\n";
        let folded = crate::load::fold_inline_tables(text);
        assert!(
            !folded.contains("# как показывать проценты }"),
            "комментарий остался внутри скобок:\n{folded}"
        );
        assert!(
            folded.contains("history_usage = { percent = \"{used}\", number = \"{remaining}\" }"),
            "таблица не свернулась:\n{folded}"
        );
        assert!(
            folded.contains("# как показывать проценты"),
            "комментарий потерялся:\n{folded}"
        );
        assert!(
            folded.contains("# как показывать числа"),
            "второй комментарий потерялся:\n{folded}"
        );

        // И такой файл разбирается.
        let loaded = crate::load::parse(ConfigFile::Global, &folded);
        assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    }

    #[test]
    fn folding_stops_where_the_table_stops() {
        // Сворачивается только таблица: всё, что после неё, остаётся как
        // было, включая переносы строк и вложенные фигурные скобки.
        let text = "a = 1\nb = {\n  x = 2\n}\nc = [1, 2]\nd = 4\n";
        let folded = crate::load::fold_inline_tables(text);
        assert_eq!(folded, "a = 1\nb = { x = 2 }\nc = [1, 2]\nd = 4\n");
    }

    #[test]
    fn a_brace_outside_a_value_is_not_folded() {
        // Скобка не на месте значения — не таблица, и сворачивать её незачем:
        // такой текст разбор и так примет.
        let text = "name = \"скобка } внутри строки\"\nпосле = 2\n";
        let folded = crate::load::fold_inline_tables(text);
        assert_eq!(folded, text);
    }

    #[test]
    fn a_nested_table_is_folded_as_a_whole() {
        let text = "a = {\n  b = {\n    c = 1\n  }\n}\nafter = 2\n";
        let folded = crate::load::fold_inline_tables(text);
        assert_eq!(folded, "a = { b = { c = 1 } }\nafter = 2\n");
        // Ключи выдуманные, поэтому проверяем не схему, а сам TOML.
        assert!(
            folded.parse::<toml_edit::DocumentMut>().is_ok(),
            "свёрнутое не разбирается: {folded}"
        );
    }

    #[test]
    fn the_generated_config_shows_a_table_line_by_line() {
        let text = render(ConfigFile::Global, &plain());
        assert!(
            text.contains("history_usage = {\n"),
            "таблица в конфиге слита в одну строку:\n{text}"
        );
    }

    #[test]
    fn comments_are_translated() {
        // Комментарий в конфиге — такой же текст для пользователя, как и всё
        // остальное, значит он переводится. Надпись о неназначенном значении
        // проверяется на файле, где такие ключи ещё есть: в `config.toml` все
        // ключи подключены, и она там не появляется.
        let text = render(ConfigFile::Edit, &russian());
        assert!(
            text.contains("Правок в памяти до снапшота."),
            "перевод комментария не попал в файл: {text}"
        );
        assert!(
            text.contains("от 0 до 65535"),
            "перевод диапазона не попал: {text}"
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
            NOT_USED,
            "Edits kept in memory before taking a snapshot.",
            "Snapshots of one file to keep, oldest dropped first. Zero means memory only.",
            "Keep snapshots between runs.",
            "Interface language. File name in the lang folder.",
            "Colours and attributes. File name in the theme folder.",
            "Keys with two meanings, and how long a press counts as held, in milliseconds.",
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
        assert_eq!(loaded.config.u16_or_default(key), 8192);
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

#[cfg(test)]
mod sn9 {
    #[test]
    fn show_parsed() {
        let c = claxis_i18n::Catalog::new();
        let text = crate::generate::render(crate::ConfigFile::Global, &c);
        println!("T>>>\n{text}");
        let loaded = crate::load::parse(crate::ConfigFile::Global, &text);
        println!("P>>> issues={:?}", loaded.issues);
        println!(
            "P>>> table={:?}",
            loaded.config.table(&crate::schema::TABLES[0])
        );
    }
}

//! Чтение языка с диска.
//!
//! Английский зашит в бинарник и всегда доступен. Остальные языки лежат
//! файлами в каталоге переводов, и читаются здесь.
//!
//! Связь односторонняя: этот крейт ничего не знает про конфиг и состояния
//! редактора. Ему говорят путь и код языка — он отдаёт каталог.

use std::path::Path;

use crate::defaults::DEFAULT_LANGUAGE;
use crate::path::LangPath;

/// Прочитать язык по коду из каталога переводов `dir`.
///
/// Папка — склад, а не выбор: файлов переводов может лежать сколько угодно,
/// и работает только тот, чей код написан в конфиге. Ничего не перебираем и
/// ничего не выбираем сами.
///
/// Крейт не знает, где лежит папка: путь приходит готовым от того, кто знает
/// раскладку на диске. Имя папки — не его дело.
///
/// Английский не читается с диска: он зашит и разбирается как «файл есть
/// всегда». Остальные языки обязаны быть на диске — молча остаться на
/// английском значило бы показать пользователю не тот интерфейс, который он
/// попросил.
pub fn load(dir: impl AsRef<Path>, lang: &str) -> crate::Result<crate::Catalog> {
    if lang == DEFAULT_LANGUAGE {
        return Ok(crate::Catalog::new());
    }
    let path = LangPath::new(dir, lang)?;
    let body = std::fs::read_to_string(path.file()).map_err(|e| crate::Error::Read {
        path: path.file().display().to_string(),
        reason: e.to_string(),
    })?;
    parse(&path, &body)
}

/// Разобрать тело файла перевода.
///
/// Каталог — плоская таблица «английская фраза = перевод». Вложенности быть
/// не может: вложенная таблица означала бы, что ключ перевода придумал не
/// код, а файл, и тогда английского текста, по которому ищется перевод,
/// просто не существует.
///
/// Повторяющийся ключ в разборе ловит сам TOML и называет строку — поэтому
/// своя проверка дублей здесь не нужна и была бы мёртвой.
fn parse(path: &LangPath, body: &str) -> crate::Result<crate::Catalog> {
    let doc: toml_edit::DocumentMut =
        body.parse()
            .map_err(|e: toml_edit::TomlError| crate::Error::Parse {
                path: path.file().display().to_string(),
                reason: e.to_string(),
            })?;

    let mut catalog = crate::Catalog::new();
    for (key, item) in doc.as_table().iter() {
        let Some(value) = item.as_str() else {
            return Err(crate::Error::NotAString {
                path: path.file().display().to_string(),
                key: key.to_string(),
            });
        };
        catalog.add(key.to_string(), value.to_string());
    }
    Ok(catalog)
}

/// Проверить, что перевод полон: нет ни одной недостающей строки.
///
/// Неполный перевод — ошибка: молча показывать английский посреди русского
/// интерфейса хуже, чем отказаться запускаться.
pub fn check_complete(
    catalog: &crate::Catalog,
    lang: &str,
    required: &[&str],
) -> crate::Result<()> {
    let missing = catalog.missing(required);
    if missing.is_empty() {
        return Ok(());
    }
    Err(crate::Error::Incomplete {
        lang: lang.to_string(),
        count: missing.len(),
        total: required.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Catalog;
    use claxis_text::temp::Temp;

    /// Что тестам нужно сверх общего каталога: файл перевода внутри него.
    trait TempLang {
        fn write(&self, name: &str, body: &str);
    }

    impl TempLang for Temp {
        fn write(&self, name: &str, body: &str) {
            std::fs::write(self.path().join(name), body).unwrap();
        }
    }

    fn good() -> &'static str {
        "\"Open file\" = \"Открыть файл\"\n\"Save file\" = \"Сохранить файл\"\n"
    }

    #[test]
    fn english_needs_no_file() {
        // Английский зашит: папки может не быть вообще.
        let dir = std::env::temp_dir().join("claxis-lang-nonexistent-xyz");
        let catalog = load(&dir, "en").unwrap();
        assert!(catalog.is_empty(), "английский зашит, а не прочитан");
    }

    #[test]
    fn a_language_is_read_from_disk() {
        let tmp = Temp::new("read");
        tmp.write("ru.toml", good());
        let catalog = load(tmp.path(), "ru").unwrap();
        assert_eq!(catalog.get("Open file"), Some("Открыть файл"));
        assert_eq!(catalog.get("Save file"), Some("Сохранить файл"));
    }

    #[test]
    fn files_in_the_folder_do_not_choose_the_language() {
        // Главное правило: папка — склад, а не выбор. Файлов переводов может
        // лежать сколько угодно, и работает только тот, чей код написан в
        // конфиге. Не тот, что попался в каталоге первым.
        let tmp = Temp::new("no-choice");
        tmp.write("ru.toml", good());
        tmp.write("de.toml", good());
        tmp.write("pt-br.toml", good());
        // Читается именно запрошенный.
        let catalog = load(tmp.path(), "de").unwrap();
        assert_eq!(catalog.get("Open file"), Some("Открыть файл"));
        // Английский зашит и не читается, сколько бы файлов ни лежало.
        assert!(load(tmp.path(), "en").unwrap().is_empty());
    }

    #[test]
    fn a_broken_unused_translation_does_not_affect_anything() {
        // Сломанный перевод того языка, который не выбран, — не помеха:
        // работает только написанный в конфиге.
        let tmp = Temp::new("broken-unused");
        tmp.write("ru.toml", good());
        tmp.write("de.toml", "\"Open file\" = \n");
        let catalog = load(tmp.path(), "ru").unwrap();
        assert_eq!(catalog.get("Open file"), Some("Открыть файл"));
    }

    #[test]
    fn missing_file_is_an_error_not_silence() {
        // Молча остаться на английском — значит показать не тот интерфейс,
        // который пользователь попросил.
        let tmp = Temp::new("missing");
        let err = load(tmp.path(), "ru").unwrap_err();
        assert!(
            matches!(err, crate::Error::Read { .. }),
            "получено: {err:?}"
        );
    }

    #[test]
    fn broken_toml_reports_what_and_where() {
        let tmp = Temp::new("broken");
        tmp.write("ru.toml", "\"Open file\" = \n");
        let err = load(tmp.path(), "ru").unwrap_err();
        assert!(
            matches!(err, crate::Error::Parse { .. }),
            "получено: {err:?}"
        );
        let text = err.text();
        assert!(text.contains("ru.toml"), "в тексте нет пути: {text}");
    }

    #[test]
    fn value_must_be_a_string() {
        let tmp = Temp::new("notstring");
        tmp.write("ru.toml", "\"Open file\" = 42\n");
        let err = load(tmp.path(), "ru").unwrap_err();
        assert!(
            matches!(err, crate::Error::NotAString { .. }),
            "получено: {err:?}"
        );
        assert!(err.text().contains("Open file"), "в тексте нет ключа");
    }

    #[test]
    fn table_where_a_string_belongs_is_an_error() {
        // Вложенность означала бы, что ключ придумал файл, а не код.
        let tmp = Temp::new("table");
        tmp.write("ru.toml", "[General]\n\"Open file\" = \"Открыть\"\n");
        let err = load(tmp.path(), "ru").unwrap_err();
        assert!(
            matches!(err, crate::Error::NotAString { .. }),
            "получено: {err:?}"
        );
        assert!(err.text().contains("General"), "в тексте нет ключа секции");
    }

    #[test]
    fn repeated_key_is_an_error_naming_the_line() {
        let tmp = Temp::new("dup");
        tmp.write(
            "ru.toml",
            "\"Open file\" = \"Открыть\"\n\"Open file\" = \"Открыть и посмотреть\"\n",
        );
        let err = load(tmp.path(), "ru").unwrap_err();
        assert!(
            matches!(err, crate::Error::Parse { .. }),
            "получено: {err:?}"
        );
        let text = err.text();
        assert!(
            text.contains("Open file") && text.contains("line 2"),
            "в тексте нет ключа и строки: {text}"
        );
    }

    #[test]
    fn language_code_cannot_escape_the_directory() {
        let tmp = Temp::new("escape");
        let err = load(tmp.path(), "../../etc/passwd").unwrap_err();
        assert!(
            matches!(err, crate::Error::UnknownLang { .. }),
            "получено: {err:?}"
        );
    }

    #[test]
    fn completeness_is_checked_against_the_code() {
        let full = Catalog::from_entries([
            ("a".to_string(), "а".to_string()),
            ("b".to_string(), "б".to_string()),
        ]);
        check_complete(&full, "ru", &["a", "b"]).unwrap();

        let partial = Catalog::from_entries([("a".to_string(), "а".to_string())]);
        let err = check_complete(&partial, "ru", &["a", "b"]).unwrap_err();
        match err {
            crate::Error::Incomplete { count, total, .. } => {
                assert_eq!((count, total), (1, 2));
            }
            other => panic!("ожидалась неполнота, получено: {other:?}"),
        }
    }
}

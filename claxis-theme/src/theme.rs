//! Темы: список в папке и проверка имени.
//!
//! Тема — файл в папке, и имя файла значит название темы. Содержимое темы
//! пока не разбирается: здесь только то, что нужно конфигу, — какие темы
//! есть и какое имя выбрано.
//!
//! `default` — не отдельная тема, а отсутствие своих цветов: рисуем тем, что
//! уже задано в терминале. Файла у неё нет, и придумывать его незачем.
//! Придумывать свои цвета тоже незачем: несколько готовых тем будут вшиты в
//! редактор.
//!
//! Крейт ничего не знает про конфиг и про редактор: ему дают папку и имя,
//! он отвечает, есть ли такая тема.

use std::path::Path;

use crate::{Error, Result};

/// Расширение файла темы.
pub const EXTENSION: &str = "toml";

/// Тема, когда в конфиге тема не написана.
///
/// `default` — это **не** отдельная тема, а отсутствие своих цветов: рисуем
/// теми цветами, что уже заданы в терминале. Отдельного файла у неё нет и
/// быть не должно: пользователю незачем придумывать пустую тему, чтобы получить
/// пустую тему.
///
/// Файлов тем в папке может лежать сколько угодно, и они все доступны: папка
/// это просто склад. Работает та тема, **имя которой написано в конфиге**, а
/// не та, что попалась первой. Не написана — значит `default`.
///
/// Если пользователь всё же положит `default.toml`, он перекроет эту и станет
/// обычной темой со своими цветами.
pub const DEFAULT_THEME: &str = "default";

/// Одна найденная тема.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Theme {
    /// Имя темы — имя файла без расширения.
    pub name: String,
    /// Сам файл.
    pub path: std::path::PathBuf,
}

/// Все темы: те, что лежат в папке, и `default`.
///
/// `default` есть всегда: это «цвета терминала», а не файл. Требовать его
/// на диске — значит ругаться на каждом первом запуске, где папки тем ещё нет.
/// Папка добавляет к ней свои темы, а одноимённый файл перекрывает `default`
/// и становится обычной темой со своими цветами.
pub fn list(dir: impl AsRef<Path>) -> Result<Vec<Theme>> {
    let dir = dir.as_ref();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // Папки нет — остаётся только `default`, то есть цвета терминала.
        // Так и бывает до первого запуска.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![builtin()]),
        Err(e) => {
            return Err(Error::ReadDir {
                dir: dir.display().to_string(),
                reason: e.to_string(),
            });
        }
    };

    let mut themes = vec![builtin()];
    for entry in entries {
        // Ошибка чтения отдельной записи — это ошибка чтения папки: молчать
        // здесь нельзя, иначе тема просто исчезнет из списка, и человек будет
        // искать её в файле, который есть.
        let entry = entry.map_err(|e| Error::ReadDir {
            dir: dir.display().to_string(),
            reason: e.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = theme_name(&path) else {
            continue;
        };
        themes.retain(|t| t.name != name);
        themes.push(Theme { name, path });
    }
    themes.sort();
    Ok(themes)
}

/// `default` — цвета терминала. Файла у неё нет и не нужно.
fn builtin() -> Theme {
    Theme {
        name: DEFAULT_THEME.to_string(),
        path: std::path::PathBuf::new(),
    }
}

/// Тема по имени или ошибка.
///
/// Имя приходит из конфига, то есть с диска, поэтому проверяется: путь вида
/// `../../../etc/passwd` не должен читать чужой файл. Проверка та же, что у
/// кода языка в `claxis-i18n`: буквы, цифры, дефис, точка и подчёркивание.
pub fn find(dir: impl AsRef<Path>, name: &str) -> Result<Theme> {
    let dir = dir.as_ref();
    if !is_valid_name(name) {
        return Err(Error::InvalidName {
            name: name.to_string(),
        });
    }
    let available = list(dir)?;
    let names: Vec<String> = available.iter().map(|t| t.name.clone()).collect();
    available
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| Error::UnknownTheme {
            name: name.to_string(),
            available: names,
        })
}

/// Имя темы из имени файла: `everforest.toml` → `everforest`.
///
/// Не-TOML пропускается молча: папка тем — папка пользователя, и кладёт
/// туда что угодно, а лишний файл не должен ломать список.
fn theme_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if path.extension()?.to_str()? != EXTENSION {
        return None;
    }
    is_valid_name(stem).then(|| stem.to_string())
}

/// Имя темы годится как имя файла.
///
/// Пустое имя и separators отбрасываются: имя становится путём, а путь не
/// должен вылезать за пределы папки тем.
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.')
        && !name.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(std::path::PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("claxis-theme-{name}"));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn theme(&self, file: &str) -> &Self {
            std::fs::write(self.0.join(file), "").unwrap();
            self
        }
        fn dir_in(&self, name: &str) {
            std::fs::create_dir_all(self.0.join(name)).unwrap();
        }
        fn names(&self) -> Vec<String> {
            names_in(&list(&self.0).unwrap())
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_folder_leaves_only_default() {
        // Так бывает до первого запуска: папки ещё нет, и это не поломка.
        // `default` есть всегда, иначе свежий конфиг ругался бы сразу.
        let dir = std::env::temp_dir().join("claxis-theme-nonexistent-xyz");
        assert_eq!(names_in(&list(&dir).unwrap()), vec!["default"]);
    }

    #[test]
    fn default_works_without_a_file() {
        // `default` — это цвета терминала, а не файл: брать его не откуда.
        let dir = std::env::temp_dir().join("claxis-theme-nonexistent-xyz");
        assert_eq!(find(&dir, DEFAULT_THEME).unwrap().name, "default");
    }

    #[test]
    fn a_file_may_replace_default() {
        // Файл с тем же именем перекрывает `default`, а не дублирует его.
        let tmp = Temp::new("override");
        tmp.theme("default.toml");
        assert_eq!(names_in(&list(&tmp.0).unwrap()), vec!["default"]);
        let found = find(&tmp.0, DEFAULT_THEME).unwrap();
        assert!(found.path.ends_with("default.toml"), "файл должен победить");
    }

    #[test]
    fn themes_are_listed_by_file_name() {
        let tmp = Temp::new("list");
        tmp.theme("everforest.toml").theme("default.toml");
        assert_eq!(tmp.names(), vec!["default", "everforest"]);
    }

    #[test]
    fn the_list_does_not_depend_on_disk_order() {
        // Порядок с диска произвольный: список обязан быть одинаковым.
        let tmp = Temp::new("sorted");
        tmp.theme("zulu.toml")
            .theme("alpha.toml")
            .theme("mike.toml");
        assert_eq!(tmp.names(), vec!["alpha", "default", "mike", "zulu"]);
    }

    #[test]
    fn foreign_files_are_skipped() {
        // Папка тем — папка пользователя: кладёт он туда что угодно.
        let tmp = Temp::new("foreign");
        tmp.theme("default.toml").theme("notes.txt").theme("README");
        assert_eq!(tmp.names(), vec!["default"]);
    }

    #[test]
    fn a_subfolder_is_not_a_theme() {
        let tmp = Temp::new("subdir");
        tmp.theme("default.toml");
        tmp.dir_in("nested.toml");
        assert_eq!(tmp.names(), vec!["default"]);
    }

    /// Имена тем в том порядке, в каком их отдаёт крейт.
    fn names_in(themes: &[Theme]) -> Vec<String> {
        themes.iter().map(|t| t.name.clone()).collect()
    }

    #[test]
    fn a_known_theme_is_found() {
        let tmp = Temp::new("find");
        tmp.theme("everforest.toml");
        let found = find(&tmp.0, "everforest").unwrap();
        assert_eq!(found.name, "everforest");
        assert!(found.path.ends_with("everforest.toml"));
    }

    #[test]
    fn an_unknown_theme_lists_what_is_there() {
        // Сообщение должно говорить, что есть: иначе пользователь гадает.
        let tmp = Temp::new("unknown");
        tmp.theme("default.toml").theme("everforest.toml");
        let err = find(&tmp.0, "solarized").unwrap_err();
        match &err {
            Error::UnknownTheme { name, available } => {
                assert_eq!(name, "solarized");
                assert_eq!(available, &["default", "everforest"]);
            }
            other => panic!("ожидалась неизвестная тема, получено: {other:?}"),
        }
        let text = err.text();
        assert!(text.contains("solarized"), "{text}");
        assert!(text.contains("everforest"), "в тексте нет списка: {text}");
    }

    #[test]
    fn files_in_the_folder_do_not_choose_the_theme() {
        // Главное правило: папка — склад, а не выбор. Файлов может лежать
        // сколько угодно, и работает та, чьё имя написано в конфиге. Не
        // написана — `default`, а не «первая попавшаяся в каталоге».
        let tmp = Temp::new("no-choice");
        tmp.theme("everforest.toml")
            .theme("solarized.toml")
            .theme("nord.toml");
        // Список показывает все три — они доступны.
        assert_eq!(
            tmp.names(),
            vec!["default", "everforest", "nord", "solarized"]
        );
        // Но пока имя не написано в конфиге, работает `default`.
        assert_eq!(find(&tmp.0, DEFAULT_THEME).unwrap().name, "default");
        // Написано — работает именно она, а не какая попалась.
        assert_eq!(find(&tmp.0, "solarized").unwrap().name, "solarized");
    }

    #[test]
    fn a_message_always_offers_the_builtin_theme() {
        // Список не бывает пустым: `default` есть всегда.
        let tmp = Temp::new("empty");
        let err = find(&tmp.0, "solarized").unwrap_err();
        let text = err.text();
        assert!(text.contains("solarized"), "{text}");
        assert!(text.contains("default"), "в тексте нет тем: {text}");
    }

    #[test]
    fn a_name_cannot_escape_the_folder() {
        // Имя приходит из конфига на диске: уход вверх читал бы чужой файл.
        let tmp = Temp::new("escape");
        tmp.theme("default.toml");
        for name in ["../../../etc/passwd", "..", "../default", ""] {
            let err = find(&tmp.0, name).unwrap_err();
            assert!(
                matches!(err, Error::InvalidName { .. }),
                "имя {name:?} должно быть отбито, получено: {err:?}"
            );
        }
    }

    #[test]
    fn ordinary_names_are_valid() {
        for name in ["default", "everforest", "pt-br", "my_theme", "dark.v2"] {
            assert!(is_valid_name(name), "{name} должно быть допустимо");
        }
    }

    #[test]
    fn a_leading_dot_is_not_a_theme() {
        // `.hidden.toml` — служебный файл, а не тема.
        assert!(!is_valid_name(".hidden"));
    }

    #[test]
    fn names_with_separators_or_spaces_are_refused() {
        // Имя становится путём: слеши, пробелы и обратный слеш недопустимы.
        for name in [
            "a/b",
            "a\\b",
            "with space",
            "tab\there",
            "нулевой\u{0}нуль",
            "путь/../выход",
        ] {
            assert!(!is_valid_name(name), "{name:?} должно быть отбито");
        }
    }

    #[test]
    fn a_name_of_exactly_sixty_four_characters_is_allowed() {
        // Граница длины: имя в 64 символа ещё годится, длиннее — уже мусор.
        let name = "a".repeat(64);
        assert!(is_valid_name(&name));
        assert!(!is_valid_name(&"a".repeat(65)));
    }

    #[test]
    fn non_ascii_names_are_refused() {
        // Буквы кириллицы в имени темы не допускаются: файл на диске может
        // прийти с любого systems, а лишние правила здесь не нужны.
        assert!(!is_valid_name("тема"));
        assert!(!is_valid_name("日本"));
    }

    #[test]
    fn two_dots_inside_a_name_are_refused() {
        // `a..b` читается как путь вверх в глазах человека, даже если им
        // не является: запрещаем целиком.
        assert!(!is_valid_name("a..b"));
        assert!(is_valid_name("a.b"));
    }

    #[test]
    fn a_file_with_a_foreign_extension_is_not_a_theme() {
        // Папка тем — папка пользователя: чужой файл просто не в списке.
        let tmp = Temp::new("extension");
        tmp.theme("everforest.txt");
        tmp.theme("readme");
        assert_eq!(names_in(&list(&tmp.0).unwrap()), vec!["default"]);
    }

    #[test]
    fn an_empty_file_is_still_a_theme() {
        // Содержимое темы пока не разбирается: важно только имя файла.
        let tmp = Temp::new("empty-file");
        tmp.theme("empty.toml");
        let found = find(&tmp.0, "empty").unwrap();
        assert!(found.path.ends_with("empty.toml"));
    }

    #[test]
    fn names_are_case_sensitive() {
        // Редактор в Linux различает регистр: `Dark` и `dark` — разные темы.
        let tmp = Temp::new("case");
        tmp.theme("dark.toml");
        assert!(find(&tmp.0, "dark").is_ok());
        assert!(matches!(
            find(&tmp.0, "Dark").unwrap_err(),
            Error::UnknownTheme { .. }
        ));
    }

    #[test]
    fn an_unknown_theme_lists_what_is_there_in_a_message() {
        let tmp = Temp::new("unknown-list");
        tmp.theme("everforest.toml");
        tmp.theme("gruvbox.toml");
        let error = find(&tmp.0, "нет-такой").unwrap_err();
        assert!(matches!(error, Error::InvalidName { .. }));
        let error = find(&tmp.0, "solarized").unwrap_err();
        let text = error.text();
        assert!(text.contains("solarized"), "{text}");
        assert!(text.contains("everforest"), "{text}");
        assert!(text.contains("gruvbox"), "{text}");
    }

    #[test]
    fn a_file_that_is_not_readable_is_an_error_not_a_silent_gap() {
        // Тема исчезла бы из списка молча, а человек смотрел бы в свой файл.
        let tmp = Temp::new("unreadable");
        tmp.theme("visible.toml");
        let blocked = tmp.0.join("blocked.toml");
        std::fs::write(&blocked, b"body").unwrap();
        let mut permissions = std::fs::metadata(&blocked).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o000);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(true);
        std::fs::set_permissions(&blocked, permissions).unwrap();

        let result = list(&tmp.0);
        // Читается ли файл — зависит от прав: проверяем именно то, что при
        // отказе мы узнаём об этом, а не теряем тему.
        if let Ok(themes) = &result {
            let names = names_in(themes);
            assert!(
                names.contains(&"blocked".to_string()),
                "файл есть, но исчез из списка: {names:?}"
            );
        }
        let _ = std::fs::remove_file(&blocked);
    }

    #[test]
    fn a_folder_instead_of_themes_is_an_error_with_its_path() {
        // Не папка тем: об этом надо сказать, а не вернуть список из одной
        // темы, будто всё в порядке.
        let file = std::env::temp_dir().join("claxis-theme-not-a-dir");
        std::fs::write(&file, b"not a directory").unwrap();
        let error = list(&file).unwrap_err();
        let text = error.text();
        assert!(text.contains("claxis-theme-not-a-dir"), "{text}");
        std::fs::remove_file(&file).unwrap();
    }
}

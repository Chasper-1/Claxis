//! Ручки крейтов.
//!
//! Один файл на крейт. Каждый файл отдаёт наружу только то, что крейт
//! предоставляет другим. Сами вызовы сюда не приходят: перенаправлением
//! занимается маршрутизатор, `crate::router`.

pub mod buffer;
pub mod commands;
pub mod config;
pub mod core;
pub mod disk;
pub mod i18n;
pub mod input;
pub mod term;
pub mod text;
pub mod theme;

#[cfg(test)]
mod tests {
    //! Проверки слоя ручек.
    //!
    //! Смысл: забыть ручку легко, и тогда крейт начнут брать напрямую, связи
    //! расползутся по кодам, и найти их станет невозможно. Обе проверки ловят
    //! это автоматически, а не памятью агента.

    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
    }

    /// Объявлен ли модуль в самом `api/mod.rs`.
    ///
    /// Берём настоящий текст файла: объявление — это строка `pub mod имя;`,
    /// и написать его руками в проверке значило бы проверять не то.
    fn module_is_declared(module: &str) -> bool {
        include_str!("mod.rs")
            .lines()
            .map(str::trim)
            .any(|line| line == format!("pub mod {module};"))
    }

    #[test]
    fn every_crate_has_a_handle() {
        let mut missing = Vec::new();
        for name in crate_names() {
            // Файл ручки называется без префикса крейта: claxis-text → text.rs
            let short = name.strip_prefix("claxis-").unwrap_or(&name);
            let handle = root().join("src/api").join(format!("{short}.rs"));
            if !handle.exists() {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "у крейтов нет ручки в src/api: {missing:?}"
        );
    }

    #[test]
    fn every_handle_is_reachable_from_the_module() {
        // Ручка лежит на диске, но не подключена: крейт формально есть, а
        // взять его нечем.
        let mut orphans = Vec::new();
        for entry in std::fs::read_dir(root().join("src/api")).expect("папка читается")
        {
            let entry = entry.expect("запись читается");
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".rs") || name == "mod.rs" {
                continue;
            }
            let module = name.trim_end_matches(".rs").to_string();
            if !module_is_declared(&module) {
                orphans.push(module);
            }
        }
        orphans.sort();
        assert!(
            orphans.is_empty(),
            "файлы ручек не объявлены в api: {orphans:?}"
        );
    }

    #[test]
    fn nothing_imports_crates_directly() {
        // Ручки — единственный вход для бинарного крейта. Крейт, к которому
        // пошли мимо src/api, связан в обход маршрутизатора.
        let mut offenders = Vec::new();
        for entry in walk(&root().join("src")) {
            // Внутри src/api сами ручки и обязаны называть крейт.
            if entry.starts_with(root().join("src/api")) {
                continue;
            }
            let text = std::fs::read_to_string(&entry).expect("файл читается");
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.starts_with("//") {
                    continue;
                }
                // Ищем именно `claxis_` с подчёркиванием: это имя крейта в
                // коде. С дефисом (`claxis-buffer`) — текст для пользователя, а
                // не вызов, и в строках он нужен.
                if mentions_a_crate(line) {
                    let rel = entry.strip_prefix(root()).unwrap_or(&entry);
                    offenders.push(format!("{}:{}: {line}", rel.display(), n + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "обход маршрутизатора, всё через crate::api:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn a_line_mentioning_a_crate_is_recognized_anywhere_in_it() {
        // Обращение может стоять в любом месте выражения: и перед ним, и после
        // ничего похожего на `use` не будет.
        for line in [
            "use claxis_i18n::Catalog;",
            "let c = claxis_text::substitute(text, &values);",
            "pub fn take(hold: &claxis_input::Hold) {}",
            "map(|v| claxis_theme::find(dir, v))",
        ] {
            assert!(mentions_a_crate(line), "не узнано: {line}");
        }
    }

    #[test]
    fn a_line_without_a_crate_is_left_alone() {
        // Ложное срабатывание хуже пропуска: проверка должна молчать о
        // строках, где крейтов нет. Комментарии пропускает вызывающий, здесь
        // проверяется сама узнаваемость имени.
        for line in [
            "let name = \"claxis-buffer\";",
            "let name = \"claxis config\";",
            "use crate::router::Editor;",
            "",
            "/// Слово на границе: claxis",
            "claxis",
        ] {
            assert!(!mentions_a_crate(line), "ложно поймано: {line}");
        }
    }

    #[test]
    fn a_crate_inside_a_comment_is_left_to_the_caller() {
        // Сама узнаваемость здесь ни при чём: строка про крейт в комментарии
        // отсекается на уровне вызывающего, который и пропускает `//`.
        let line = "// claxis_buffer::Buffer — это в тексте";
        assert!(mentions_a_crate(line));
        let check = |line: &str| !line.trim().starts_with("//") && mentions_a_crate(line);
        assert!(!check(line));
    }

    #[test]
    fn a_prefix_without_a_crate_name_is_not_a_crate() {
        // `claxis_` с пустым именем после — не обращение к крейту.
        assert!(!mentions_a_crate("let x = claxis_ ;"));
        assert!(!mentions_a_crate("claxis_"));
    }

    #[test]
    fn only_the_disk_crate_touches_the_disk() {
        // Работа с диском в проекте одна. Если второй крейт начнёт читать или
        // писать сам, правило разъедется: у него будут свои пути, своя
        // атомарность и своя обработка ошибок, а правка одной не дойдёт до
        // другой.
        let mut offenders = Vec::new();
        let mut sources = Vec::new();
        for crate_name in crate_names() {
            // Диску и конфигу положено: первый делает работу, второй знает
            // раскладку. Остальным — только через них.
            if crate_name == "claxis-disk" || crate_name == "claxis-config" {
                continue;
            }
            sources.push(root().join(&crate_name).join("src"));
        }
        // Бинарник — такой же код, как и крейты: правило одно на всех.
        sources.push(root().join("src"));
        for src in sources {
            for entry in walk(&src) {
                let text = std::fs::read_to_string(&entry).expect("файл читается");
                // Всё после `#[cfg(test)]` — тесты, а тестам положено писать
                // файлы: иначе нечего проверять.
                let test_start = text
                    .lines()
                    .position(|l| l.trim() == "#[cfg(test)]")
                    .unwrap_or(usize::MAX);
                for (n, line) in text.lines().enumerate() {
                    let line = line.trim();
                    // Комментарий — не запись, а пример в тесте — не запись
                    // крейта.
                    if line.starts_with("//") || n >= test_start {
                        continue;
                    }
                    for call in [
                        "fs::write",
                        "fs::remove_file",
                        "fs::rename",
                        "fs::create_dir_all",
                        "fs::remove_dir_all",
                        "fs::read_to_string",
                        "fs::read_dir",
                        "fs::metadata",
                    ] {
                        if line.contains(call) {
                            let rel = entry.strip_prefix(root()).unwrap_or(&entry);
                            offenders.push(format!("{}:{}: {line}", rel.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "пишут на диск мимо claxis-disk:\n{}",
            offenders.join("\n")
        );
    }

    /// Строка кода называет крейт воркспейса.
    ///
    /// Имя крейта — это идентификатор `claxis_` плюс буквы, цифры и
    /// подчёркивания. Проверяется ровно это, а не отдельные ключевые слова
    /// вроде `use` или `::`: обращение может стоять в любом месте выражения,
    /// и любая другая проверка пропускает часть обходов.
    fn mentions_a_crate(line: &str) -> bool {
        let bytes = line.as_bytes();
        let prefix = b"claxis_";
        bytes.windows(prefix.len()).enumerate().any(|(offset, w)| {
            w == prefix
                && bytes
                    .get(offset + prefix.len())
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
        })
    }

    /// Имена всех крейтов воркспейса.
    fn crate_names() -> Vec<String> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(root())
            .expect("корень читается")
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("claxis-") && entry.path().join("src").is_dir() {
                names.push(name);
            }
        }
        names.sort();
        names
    }

    /// Все файлы исходников бинарного крейта.
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        out
    }
}

//! Проверки слоя ручек.
//!
//! Смысл: забыть ручку легко, и тогда крейт начнут брать напрямую, связи
//! расползутся по кодам, и найти их станет невозможно. Обе проверки ловят это
//! автоматически, а не памятью агента.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
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
fn nothing_imports_crates_directly() {
    // Ручки — единственный вход для бинарного крейта. Крейт, к которому пошли
    // мимо src/api, связан в обход маршрутизатора.
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
            if line.contains("use claxis_") || line.contains("::claxis_") {
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

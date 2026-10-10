//! Загрузка конфига с указанием, что именно не так и где.
//!
//! Загрузка никогда не падает целиком. Ошибка в одной секции не должна
//! отключать остальные: пользователь потерял бы настройки, которые у него в
//! порядке. Поэтому плохая секция отбрасывается, остальные работают, а
//! список проблем возвращается вместе с конфигом.
//!
//! Проблема несёт файл и строку. Это нужно, чтобы открыть конфиг и поставить
//! курсор ровно на то место, которое надо исправить.

use super::Config;
use super::schema::{ConfigFile, KEYS, KeyDef};

/// Что не так с ключом.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Ключа нет в схеме: опечатка или настройка из будущей версии.
    UnknownKey,
    /// Значение не того типа.
    BadType {
        /// Что ожидалось.
        expected: &'static str,
        /// Что пришло на самом деле.
        got: String,
    },
}

/// Проблема в конфиге: файл, строка, ключ и суть.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Файл, в котором проблема.
    pub file: &'static str,
    /// Строка в файле, начиная с 1.
    pub line: usize,
    /// Ключ: `General.history_depth`.
    pub key: String,
    /// В чём проблема.
    pub problem: Problem,
}

impl Issue {
    /// Текст для сообщения: файл, строка, ключ и суть.
    ///
    /// Собирается из частей, а не хранится готовой строкой: пользователь
    /// переводит это на свой язык.
    pub fn text(&self) -> String {
        let where_ = format!("{}:{}", self.file, self.line);
        match &self.problem {
            Problem::UnknownKey => format!("{where_}: unknown key {}", self.key),
            Problem::BadType { expected, got } => {
                format!("{where_}: key {} has {got}, expected {expected}", self.key)
            }
        }
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

/// Результат загрузки: конфиг и список проблем.
///
/// Конфиг всегда пригоден к работе: плохие секции в него не попали.
#[derive(Debug, Default)]
pub struct Loaded {
    /// Конфиг без плохих секций.
    pub config: Config,
    /// Что не так: файл, строка, ключ.
    pub issues: Vec<Issue>,
}

impl Loaded {
    /// Всё в порядке.
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// Первая проблема: на неё ставится курсор при открытии файла.
    pub fn first(&self) -> Option<&Issue> {
        self.issues.first()
    }
}

/// Разобрать один файл конфига.
///
/// Не падает: плохие секции отбрасываются, проблемы возвращаются списком.
pub fn parse(file: ConfigFile, text: &str) -> Loaded {
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(doc) => doc,
        Err(e) => {
            return Loaded {
                config: Config::new(),
                issues: vec![Issue {
                    file: file.file_name(),
                    line: 1,
                    key: String::new(),
                    problem: Problem::BadType {
                        expected: "a valid file",
                        got: e.message().to_string(),
                    },
                }],
            };
        }
    };

    let mut issues = Vec::new();
    let mut bad_sections: Vec<String> = Vec::new();

    for (section_name, item) in doc.as_table().iter() {
        let Some(section) = item.as_table() else {
            continue;
        };
        for (key_name, value) in section.iter() {
            let path = format!("{section_name}.{key_name}");
            match super::schema::find(file, &path) {
                None => {
                    bad_sections.push(section_name.to_string());
                    issues.push(Issue {
                        file: file.file_name(),
                        line: line_of(text, section_name, key_name).unwrap_or(0),
                        key: path,
                        problem: Problem::UnknownKey,
                    });
                }
                Some(def) => {
                    if let Err(problem) = check(def, value) {
                        bad_sections.push(section_name.to_string());
                        issues.push(Issue {
                            file: file.file_name(),
                            line: line_of(text, section_name, key_name).unwrap_or(0),
                            key: path,
                            problem,
                        });
                    }
                }
            }
        }
    }

    // Плохая секция целиком выключается: остальные настройки из неё могут
    // оказаться неверными из-за той же опечатки.
    bad_sections.sort_unstable();
    bad_sections.dedup();
    let mut doc = doc;
    for section in &bad_sections {
        doc.as_table_mut().remove(section);
    }

    Loaded {
        config: Config::from_documents([(file, doc)]),
        issues,
    }
}

/// Разобрать все файлы: плохая секция в одном файле не трогает другие.
pub fn parse_all(files: impl IntoIterator<Item = (ConfigFile, String)>) -> Loaded {
    let mut all = Loaded::default();
    let mut docs = Vec::new();
    for (file, text) in files {
        let one = parse(file, &text);
        if let Some(doc) = one.config.doc(file) {
            docs.push((file, doc.clone()));
        }
        all.issues.extend(one.issues);
    }
    all.config = Config::from_documents(docs);
    all
}

/// Значение соответствует типу ключа?
fn check(def: &KeyDef, value: &toml_edit::Item) -> std::result::Result<(), Problem> {
    use super::schema::Kind;
    let ok = match def.kind {
        Kind::Bool => value.as_bool().is_some(),
        Kind::U32 => value
            .as_integer()
            .map(|n| n >= 0 && n <= u32::MAX as i64)
            .unwrap_or(false),
        Kind::Str => value.as_str().is_some(),
    };
    if ok {
        Ok(())
    } else {
        Err(Problem::BadType {
            expected: def.kind.expected(),
            got: value.to_string().trim().to_string(),
        })
    }
}

/// Строка в исходном тексте, где задан ключ.
///
/// `toml_edit` позицию не отдаёт, поэтому строка ищется проходом по тексту.
/// Плохая секция с несколькими ключами укажет на первую найденную строку.
pub fn line_of(text: &str, section: &str, key: &str) -> Option<usize> {
    let mut current = "";
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current = trimmed.trim_start_matches('[').trim_end_matches(']');
            continue;
        }
        if current == section
            && trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == key)
        {
            return Some(i + 1);
        }
    }
    None
}

/// Все ключи файла — для генератора конфига.
pub fn keys_of(file: ConfigFile) -> impl Iterator<Item = &'static KeyDef> {
    KEYS.iter().filter(move |k| k.file == file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_of_finds_the_key() {
        let text = "[General]\nhistory_depth = 64\n\n[Word]\nindent_size = 4\n";
        assert_eq!(line_of(text, "General", "history_depth"), Some(2));
        assert_eq!(line_of(text, "Word", "indent_size"), Some(5));
        assert_eq!(line_of(text, "General", "indent_size"), None);
        assert_eq!(line_of(text, "Nope", "history_depth"), None);
    }

    #[test]
    fn clean_file_has_no_issues() {
        let loaded = parse(ConfigFile::Edit, "[General]\nhistory_depth = 64\n");
        assert!(loaded.is_ok(), "проблемы: {:?}", loaded.issues);
        assert!(loaded.first().is_none());
    }

    #[test]
    fn unknown_key_reports_file_line_and_key() {
        let loaded = parse(ConfigFile::Edit, "[General]\nhistory_dept = 64\n");
        let issue = loaded.first().expect("должна быть проблема");
        assert_eq!(issue.file, "edit.toml");
        assert_eq!(issue.line, 2);
        assert_eq!(issue.key, "General.history_dept");
        assert_eq!(issue.problem, Problem::UnknownKey);
    }

    #[test]
    fn bad_value_reports_what_was_expected() {
        let loaded = parse(ConfigFile::Edit, "[General]\nhistory_depth = \"много\"\n");
        let issue = loaded.first().unwrap();
        assert!(matches!(issue.problem, Problem::BadType { .. }));
        let text = issue.text();
        assert!(text.contains("edit.toml:2"), "{text}");
        assert!(text.contains("expected"), "{text}");
    }

    #[test]
    fn bad_section_is_dropped_but_others_survive() {
        // Одна плохая секция не должна отключать остальные.
        let text = "[General]\nhistory_depth = \"нет\"\n\n[Word]\nindent_size = 4\n";
        let loaded = parse(ConfigFile::Edit, text);
        assert!(!loaded.is_ok());
        // Плохая секция выключена.
        let bad = KEYS
            .iter()
            .find(|k| k.path == "General.history_depth")
            .unwrap();
        assert!(loaded.config.get(bad).is_none());
        // Хорошая на месте.
        assert_eq!(loaded.config.open_files(), vec![ConfigFile::Edit]);
    }

    #[test]
    fn one_bad_file_does_not_break_another() {
        let loaded = parse_all([
            (
                ConfigFile::Edit,
                "[General]\nhistory_dept = 1\n".to_string(),
            ),
            (
                ConfigFile::Files,
                "[General]\nsnapshots_keep = 7\n".to_string(),
            ),
        ]);
        assert_eq!(loaded.issues.len(), 1);
        assert_eq!(loaded.issues[0].file, "edit.toml");
        // files.toml прочитан целиком.
        let keep = KEYS
            .iter()
            .find(|k| k.path == "General.snapshots_keep")
            .unwrap();
        assert_eq!(loaded.config.u32_or_default(keep), 7);
    }

    #[test]
    fn broken_syntax_reports_the_line() {
        let loaded = parse(ConfigFile::Edit, "[General\nhistory_depth = 1\n");
        assert!(!loaded.is_ok());
        assert_eq!(loaded.issues[0].line, 1);
    }

    #[test]
    fn good_section_survives_a_bad_neighbour() {
        // В одном файле плохая и хорошая секции рядом.
        let text = "[General]\nhistory_dept = 1\n\n[Word]\nindent_size = 8\n";
        let loaded = parse(ConfigFile::Edit, text);
        assert_eq!(loaded.issues.len(), 1, "проблемы: {:?}", loaded.issues);
        let doc = loaded.config.doc(ConfigFile::Edit).unwrap();
        assert!(
            doc.get("General").is_none(),
            "плохая секция должна быть выключена"
        );
        assert!(doc.get("Word").is_some(), "хорошая секция должна остаться");
    }
}

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
    /// Файл не прочитан: `{reason}`.
    Unreadable(String),
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
    /// Текст на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn super::messages::Messages) -> String {
        messages.issue(self)
    }

    /// Текст на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&super::messages::En)
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
/// Свернуть многострочные встроенные таблицы в одну строку.
///
/// В TOML 1.0 встроенная таблица обязана помещаться в строку, а читать её так
/// невозможно: настройка на три строки сливается с соседними. Поэтому перед
/// разбором таблица сворачивается, а при записи разворачивается обратно — на
/// экране и в файле она остаётся разбитой по строкам.
///
/// Сворачивается **только таблица** — от открывающей скобки до её парной.
/// Всё остальное в файле остаётся на своих местах: переносы строк, имена,
/// комментарии и строковые значения не трогаются. Комментарий, написанный
/// внутри многострочной таблицы, не попадает в скобки — он встаёт на
/// отдельную строку сразу под таблицей и остаётся видимым.
pub fn fold_inline_tables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    // Глубина вложенности таблиц, пока идёт сворачивание.
    let mut depth = 0usize;
    // Свёрнутая таблица копится отдельно: пока она не закончилась, в `out`
    // ничего не пишется, иначе перенос строки внутри неё уехал бы наружу.
    let mut folded = String::new();
    // Комментарии изнутри таблицы: их нельзя оставить в скобках, но и терять
    // нельзя — они встают отдельными строками под таблицей.
    let mut comments: Vec<String> = Vec::new();
    // Внутри строки скобки и запятые не считаются.
    let mut quote: Option<char> = None;
    let mut escaped = false;
    // Последний значимый символ снаружи строки: по нему видно, что `{`
    // стоит именно на месте значения, а не внутри имени или комментария.
    let mut last = ' ';

    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if depth > 0 {
                folded.push(c);
            } else {
                out.push(c);
            }
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
                last = q;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                if depth > 0 {
                    folded.push(c);
                } else {
                    out.push(c);
                }
            }
            // Комментарий внутри таблицы не может остаться в скобках: TOML
            // его там не допускает. Текст не теряется — он уходит отдельной
            // строкой под таблицей, где человек его и писал.
            '#' if depth > 0 => {
                let mut comment = String::from("#");
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                    comment.push(skipped);
                }
                comments.push(comment.trim_end().to_string());
                folded.push(' ');
            }
            '{' => {
                if last == '=' || last == ',' {
                    depth += 1;
                    if depth == 1 {
                        folded.clear();
                    }
                    folded.push(c);
                } else if depth > 0 {
                    // Вложенная таблица — часть значения, сворачивается вместе.
                    folded.push(c);
                } else {
                    out.push(c);
                }
                last = c;
            }
            '}' => {
                if depth == 0 {
                    out.push(c);
                    last = c;
                    continue;
                }
                folded.push(c);
                depth -= 1;
                if depth == 0 {
                    // Таблица закончилась: вставляем её целиком, а комментарии
                    // из неё — отдельными строками сразу под ней.
                    out.push_str(&tidy_inline(&folded));
                    for comment in comments.drain(..) {
                        out.push('\n');
                        out.push_str(&comment);
                    }
                    last = '}';
                }
            }
            '\n' if depth > 0 => folded.push(' '),
            c if c.is_whitespace() => {
                if depth > 0 {
                    folded.push(c);
                } else {
                    out.push(c);
                }
            }
            c => {
                if depth > 0 {
                    folded.push(c);
                } else {
                    out.push(c);
                }
                last = c;
            }
        }
    }
    out
}

/// Убрать лишние пробелы из свёрнутой таблицы.
///
/// Переносы строк и отступы превратились в пробелы, и без этого в скобках
/// оказалось бы `{   a = 1,     b = 2   }`. Пробелы внутри строк не трогаются:
/// там они часть значения.
fn tidy_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut space = false;
    for c in text.chars() {
        if let Some(q) = quote {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if c.is_whitespace() {
            space = true;
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            _ => {}
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

pub fn parse(file: ConfigFile, text: &str) -> Loaded {
    let folded = fold_inline_tables(text);
    let doc: toml_edit::DocumentMut = match folded.parse() {
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

    walk(
        file,
        doc.as_table(),
        "",
        text,
        &mut issues,
        &mut bad_sections,
    );

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

/// Обойти все ключи файла, включая секции режимов.
///
/// Обход идёт вглубь, путь ключа собирается по дороге: секция режима даёт
/// уровень `[type]`, а ключ в ней — `type.indent_size`.
fn walk(
    file: ConfigFile,
    table: &toml_edit::Table,
    prefix: &str,
    text: &str,
    issues: &mut Vec<Issue>,
    bad_sections: &mut Vec<String>,
) {
    for (name, item) in table.iter() {
        // Путь до ключа: вложенная секция добавляет уровень, скалярное
        // значение стоит уже на своём месте.
        let here = |prefix: &str, name: &str| {
            if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}.{name}")
            }
        };

        // Вложенная таблица — это секция режима, а не ключ.
        if let Some(sub) = item.as_table() {
            let section = here(prefix, name);
            walk(file, sub, &section, text, issues, bad_sections);
            continue;
        }

        let section = prefix.to_string();
        let path = here(prefix, name);
        let mut issue_key = path.clone();
        let problem = match super::schema::find(file, &path) {
            None => {
                // Табличная настройка: путь есть в TABLES, а не в KEYS.
                // Внутри неё любые ключи — это данные, проверяем значения.
                match super::schema::find_table(file, &path) {
                    Some(table) => {
                        if let Some(inner) = item.as_table_like() {
                            let mut problem = None;
                            for (key, value) in inner.iter() {
                                if let Err(p) = check_kind(table.kind, value) {
                                    issue_key = table.path_of(key);
                                    problem = Some(p);
                                    break;
                                }
                            }
                            problem
                        } else {
                            Some(Problem::BadType {
                                expected: table.kind.expected(),
                                got: item.to_string().trim().to_string(),
                            })
                        }
                    }
                    None => Some(Problem::UnknownKey),
                }
            }
            Some(def) => check(def, item).err(),
        };
        if let Some(problem) = problem {
            // Отключается секция целиком: ключ внутри неё, а не сам ключ.
            bad_sections.push(section.clone());
            issues.push(Issue {
                file: file.file_name(),
                line: line_of(text, &section, name).unwrap_or(0),
                key: issue_key,
                problem,
            });
        }
    }
}

/// Разобрать все файлы: плохая секция в одном файле не трогает другие.
///
/// Подпорка для тестов: настоящий вход — [`crate::generate::read_all`], и он
/// разбирает файлы сам. В рабочем коде этого пути нет, чтобы правила разбора
/// не могли разойтись в двух местах.
#[cfg(test)]
fn parse_all(files: impl IntoIterator<Item = (ConfigFile, String)>) -> Loaded {
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

/// Значение соответствует типу внутри табличной настройки?
fn check_kind(
    kind: super::schema::Kind,
    item: &toml_edit::Item,
) -> std::result::Result<(), Problem> {
    let placeholder = KeyDef {
        file: super::schema::ConfigFile::Global,
        path: "",
        default: "0",
        kind,
        comment: "",
    };
    check(&placeholder, item)
}

/// Значение соответствует типу ключа?
///
/// Наружу: тем же правилом проверяется и запись значения, иначе ключ можно
/// было бы положить в конфиг в виде, который сам же разбор потом отвергнет.
pub fn value_matches(def: &KeyDef, value: &str) -> bool {
    // Список в схеме хранится текстом без скобок, поэтому для проверки он
    // снова оборачивается в них: проверять надо ровно то, что окажется в
    // конфиге, а не то, что из него выведется.
    let text = match def.kind {
        super::schema::Kind::Array => format!("v = [{value}]\n"),
        // Строка в схеме хранится без кавычек, а в TOML она обязана быть в
        // кавычках: `hello` без них — не значение, а опечатка.
        super::schema::Kind::Str | super::schema::Kind::Path => {
            format!("v = \"{value}\"\n")
        }
        _ => format!("v = {value}\n"),
    };
    let Ok(parsed) = text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(item) = parsed.get("v") else {
        return false;
    };
    check(def, item).is_ok()
}

fn check(def: &KeyDef, value: &toml_edit::Item) -> std::result::Result<(), Problem> {
    use super::schema::Kind;
    let ok = match def.kind {
        Kind::Bool => value.as_bool().is_some(),
        Kind::U16 => value
            .as_integer()
            .map(|n| (0..=u16::MAX as i64).contains(&n))
            .unwrap_or(false),
        Kind::U8 => value
            .as_integer()
            .map(|n| (0..=u8::MAX as i64).contains(&n))
            .unwrap_or(false),
        Kind::Str | Kind::Path => value.as_str().is_some(),
        // Список строк: `[]` и `["a", "b"]` годятся, `["a", 1]` — нет.
        Kind::Array => value
            .as_array()
            .is_some_and(|a| a.iter().all(|item| item.as_str().is_some())),
        Kind::Int => value.as_integer().is_some(),
        // Дробное принимается и целым: `3` — это тоже число, и запрещать его
        // в дробной настройке незачем.
        Kind::Float => value.as_float().is_some() || value.as_integer().is_some(),
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
        let text = "[General]\nhistory_depth = 64\n\n[type]\nindent_size = 4\n";
        assert_eq!(line_of(text, "General", "history_depth"), Some(2));
        assert_eq!(line_of(text, "type", "indent_size"), Some(5));
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
        let text = "[General]\nhistory_depth = \"нет\"\n\n[type]\nindent_size = 4\n";
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
        assert_eq!(loaded.config.u8_or_default(keep).unwrap(), 7);
    }

    #[test]
    fn broken_syntax_reports_the_line() {
        let loaded = parse(ConfigFile::Edit, "[General\nhistory_depth = 1\n");
        assert!(!loaded.is_ok());
        assert_eq!(loaded.issues[0].line, 1);
    }

    #[test]
    fn an_unknown_section_does_not_break_the_file() {
        // Секция, которой нет в схеме, — ошибка, но остальной файл читается.
        let text = "[General]\nhistory_depth = 64\n\n[Word]\nindent_size = 8\n";
        let loaded = parse(ConfigFile::Edit, text);
        assert_eq!(loaded.issues.len(), 1, "проблемы: {:?}", loaded.issues);
        let doc = loaded.config.doc(ConfigFile::Edit).unwrap();
        assert!(
            doc.get("General").is_some(),
            "хорошая секция должна остаться"
        );
        assert!(doc.get("Word").is_none(), "неизвестная секция выключена");
    }
}

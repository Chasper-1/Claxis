//! Схема конфига: какие ключи бывают и что каждый значит.
//!
//! # Файлы
//!
//! Конфиг разбит на файлы. Общие настройки всего редактора лежат в
//! `config.toml`. Каждое состояние — в своём: `edit.toml`, `command.toml`,
//! `files.toml`. Темы и локализация — тоже отдельно, они не принадлежат ни
//! одному состоянию.
//!
//! Имя файла в ключе не участвует: оно уже названо самим файлом. Внутри файла
//! секция `[General]` — это сам файл, остальные секции — режимы.
//!
//! ```toml
//! # config.toml — настройки всего редактора
//! [General]
//! hold = { alt = 200, ctrl = 200 }
//!
//! # edit.toml — состояние правки
//! [General]
//! history_depth = 8192
//!
//! [type]
//! indent_size = 4
//! ```
//!
//! # Ключи
//!
//! Ключи пишутся по-английски и называются максимально понятно. Сокращения
//! вроде `tmo` или `isz` запрещены: имя читает человек, и если его приходится
//! расшифровывать, оно неудачное.

/// Файл конфига.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigFile {
    /// Настройки всего редактора: `config.toml`.
    ///
    /// Сюда то, что не принадлежит ни одному состоянию: ввод, пути, выбор
    /// темы и языка.
    Global,
    /// Состояние правки: `edit.toml`.
    Edit,
    /// Командное состояние: `command.toml`.
    Command,
    /// Состояние файлов: `files.toml`.
    Files,
}

/// Папка с темами. Имя файла в ней — название темы.
pub const THEME_DIR: &str = "theme";
/// Папка с переводами. Имя файла в ней — код языка.
pub const LANG_DIR: &str = "lang";

impl ConfigFile {
    /// Три состояния, у каждого свой файл.
    pub const STATES: &'static [ConfigFile] =
        &[ConfigFile::Edit, ConfigFile::Command, ConfigFile::Files];

    /// Все файлы настроек.
    pub const ALL: &'static [ConfigFile] = &[
        ConfigFile::Global,
        ConfigFile::Edit,
        ConfigFile::Command,
        ConfigFile::Files,
    ];

    /// Имя файла без расширения.
    pub fn name(self) -> &'static str {
        match self {
            ConfigFile::Global => "config",
            ConfigFile::Edit => "edit",
            ConfigFile::Command => "command",
            ConfigFile::Files => "files",
        }
    }

    /// Имя файла с расширением.
    pub fn file_name(self) -> &'static str {
        match self {
            ConfigFile::Global => "config.toml",
            ConfigFile::Edit => "edit.toml",
            ConfigFile::Command => "command.toml",
            ConfigFile::Files => "files.toml",
        }
    }

    /// Файл относится к состоянию, а не к общим настройкам.
    pub fn is_state(self) -> bool {
        Self::STATES.contains(&self)
    }
}

/// Секция самого состояния.
pub const GENERAL: &str = "General";

/// Тип значения ключа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Да или нет.
    Bool,
    /// Целое от 0 до 65535.
    U16,
    /// Целое от 0 до 255.
    U8,
    /// Строка.
    Str,
    /// Список строк: `["position", "mode"]`.
    ///
    /// Отдельный вид, а не строка: список и строка разбираются по-разному, и
    /// пустой список в строке не спрячешь.
    Array,
    /// Путь к файлу или папке.
    Path,
    /// Целое со знаком, любой размер: `-1`, `0`, `100000`.
    Int,
    /// Вещественное число: `0.5`, `-1.25`.
    Float,
}

impl Kind {
    /// Ожидаемое значение для сообщения об ошибке.
    pub fn expected(self) -> &'static str {
        match self {
            Kind::Bool => "a boolean",
            Kind::U16 => "a number from 0 to 65535",
            Kind::U8 => "a number from 0 to 255",
            Kind::Str => "a string",
            Kind::Array => "a list of strings",
            Kind::Path => "a path",
            Kind::Int => "a whole number",
            Kind::Float => "a number with a fractional part",
        }
    }
}

/// Описание одного ключа.
#[derive(Clone, Copy, Debug)]
pub struct KeyDef {
    /// Файл-состояние, в котором ключ живёт.
    pub file: ConfigFile,
    /// Путь внутри файла: `General.history_depth` или `type.indent_size`.
    pub path: &'static str,
    /// Значение по умолчанию, как его запишет генератор.
    pub default: &'static str,
    /// Тип значения.
    pub kind: Kind,
    /// Комментарий к ключу. Он же ключ в каталоге перевода: английский текст
    /// и есть то, что переводится.
    pub comment: &'static str,
}

impl KeyDef {
    /// Секция ключа: `General` или `type`.
    pub fn section(&self) -> &str {
        match self.path.rfind('.') {
            Some(i) => &self.path[..i],
            None => "",
        }
    }

    /// Имя ключа без секции: `history_depth`.
    pub fn key(&self) -> &str {
        match self.path.rfind('.') {
            Some(i) => &self.path[i + 1..],
            None => self.path,
        }
    }

    /// Ключ лежит в секции самого состояния, а не в режиме.
    pub fn is_general(&self) -> bool {
        self.section() == GENERAL
    }
}

/// Настройка, значение которой — таблица со свободными ключами.
///
/// Обычная настройка это «один ключ — одно значение», и все такие ключи
/// перечислены в [`KEYS`]. А здесь наоборот: значение само является таблицей, и
/// **имена её ключей — это данные**. Сколько записей настроить, решает
/// пользователь, поэтому перечислить их в схеме нельзя.
#[derive(Clone, Copy, Debug)]
pub struct TableDef {
    /// Файл и путь настройки: `General.hold`.
    pub file: ConfigFile,
    /// Путь настройки внутри файла.
    pub path: &'static str,
    /// Тип значения каждого ключа внутри таблицы.
    pub kind: Kind,
    /// Значения по умолчанию: имя ключа и его значение.
    pub default: &'static [(&'static str, &'static str)],
    /// Комментарий к настройке. Он же ключ в каталоге перевода.
    pub comment: &'static str,
}

impl TableDef {
    /// Путь одного ключа внутри таблицы: `General.hold.alt`.
    pub fn path_of(&self, key: &str) -> String {
        format!("{}.{key}", self.path)
    }
}

/// Настройки, значение которых — таблица.
pub const TABLES: &[TableDef] = &[
    TableDef {
        file: ConfigFile::Global,
        path: "General.hold",
        kind: Kind::U16,
        default: &[("alt", "200"), ("ctrl", "200")],
        comment: "Keys with two meanings, and how long a press counts as held, in milliseconds.",
    },
    TableDef {
        file: ConfigFile::Global,
        path: "Savescreen.Setting.history_usage",
        kind: Kind::Str,
        default: &[("percent", "{used}"), ("number", "{remaining}")],
        comment: "How history usage is written: percent or number. remaining = limit - used.",
    },
];

/// Найти табличную настройку по файлу и пути.
pub fn find_table(file: ConfigFile, path: &str) -> Option<&'static TableDef> {
    TABLES.iter().find(|t| t.file == file && t.path == path)
}

/// Все ключи конфига.
///
/// Один список — источник правды. Генератор берёт отсюда и значения, и
/// комментарии; проверка сверяет с ним файл пользователя. Расхождение
/// невозможно, потому что список один.
pub const KEYS: &[KeyDef] = &[
    // ── config.toml — настройки всего редактора ────────────────────────
    KeyDef {
        file: ConfigFile::Global,
        path: "General.language",
        default: claxis_i18n::defaults::DEFAULT_LANGUAGE,
        kind: Kind::Str,
        comment: "Interface language. File name in the lang folder.",
    },
    KeyDef {
        file: ConfigFile::Global,
        path: "General.theme",
        default: "default",
        kind: Kind::Str,
        comment: "Colours and attributes. File name in the theme folder.",
    },
    // ── edit.toml ──────────────────────────────────────────────────────
    KeyDef {
        file: ConfigFile::Edit,
        path: "General.history_depth",
        default: "8192",
        kind: Kind::U16,
        comment: "Edits kept in memory before taking a snapshot.",
    },
    // ── files.toml ─────────────────────────────────────────────────────
    KeyDef {
        file: ConfigFile::Files,
        path: "General.snapshots_keep",
        default: "3",
        kind: Kind::U8,
        comment: "Snapshots of one file to keep, oldest dropped first. Zero means memory only.",
    },
    KeyDef {
        file: ConfigFile::Files,
        path: "General.snapshots_persist",
        default: "true",
        kind: Kind::Bool,
        comment: "Keep snapshots between runs.",
    },
    KeyDef {
        file: ConfigFile::Files,
        path: "General.snapshots_interval",
        default: "300",
        kind: Kind::U16,
        comment: "Seconds between snapshots of one document. Zero means no snapshots at all.",
    },
    // ── экран статистики после сохранения ────────────────────────────────
    //
    // Как в демо-конфиге: своя секция с подсекциями, а не плоские ключи в
    // `General`. Экран — не режим правки, а отдельная сущность со своими
    // настройками, и прятать их в общий список значит потерять этот смысл.
    KeyDef {
        file: ConfigFile::Global,
        path: "Savescreen.include",
        default: "true",
        kind: Kind::Bool,
        comment: "Show the statistics screen after saving.",
    },
    KeyDef {
        file: ConfigFile::Global,
        path: "Savescreen.mode",
        default: "fullscreen",
        kind: Kind::Str,
        comment: "How the statistics screen shows: one of fullscreen, message, window.",
    },
    KeyDef {
        file: ConfigFile::Global,
        path: "Savescreen.Statistic.input_char",
        default: "true",
        kind: Kind::Bool,
        comment: "Show how many characters were typed.",
    },
    KeyDef {
        file: ConfigFile::Global,
        path: "Savescreen.Statistic.delete_char",
        default: "true",
        kind: Kind::Bool,
        comment: "Show how many characters were deleted.",
    },
    KeyDef {
        file: ConfigFile::Global,
        path: "Savescreen.Statistic.history_usage",
        default: "true",
        kind: Kind::Bool,
        comment: "Show how much of the undo history is used.",
    },
];

/// Найти ключ по файлу и пути внутри него.
pub fn find(file: ConfigFile, path: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.file == file && k.path == path)
}

/// Найти ключ по файлу, секции и имени.
pub fn find_in(file: ConfigFile, section: &str, key: &str) -> Option<&'static KeyDef> {
    KEYS.iter()
        .find(|k| k.file == file && k.section() == section && k.key() == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_state_files() {
        // Состояний ровно три, и у каждого свой файл.
        assert_eq!(ConfigFile::STATES.len(), 3);
        let names: Vec<&str> = ConfigFile::STATES.iter().map(|s| s.file_name()).collect();
        assert_eq!(names, ["edit.toml", "command.toml", "files.toml"]);
        for file in ConfigFile::STATES {
            assert!(file.is_state());
        }
    }

    #[test]
    fn editor_wide_settings_live_in_their_own_file() {
        // Общие настройки всего редактора — в `config.toml`, а не в состоянии.
        assert_eq!(ConfigFile::Global.file_name(), "config.toml");
        assert!(!ConfigFile::Global.is_state());
        let table = find_table(ConfigFile::Global, "General.hold").unwrap();
        assert_eq!(table.path, "General.hold");
        // Табличной настройки нет ни в одном состоянии.
        for file in ConfigFile::STATES {
            assert!(
                find_table(*file, "General.hold").is_none(),
                "настройка всего редактора попала в {:?}",
                file.file_name()
            );
        }
    }

    #[test]
    fn every_file_has_a_name() {
        // Каждый файл знает своё имя, иначе его не найти на диске.
        for file in ConfigFile::ALL {
            assert!(file.file_name().ends_with(".toml"));
            assert!(!file.name().is_empty());
        }
        // Имена не повторяются.
        let mut names: Vec<&str> = ConfigFile::ALL.iter().map(|f| f.file_name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "имена файлов повторяются");
    }

    #[test]
    fn themes_and_translations_are_folders_not_settings() {
        // Тема и перевод — не файлы настроек, а файлы в папках. Имя файла
        // значит название темы или код языка.
        assert_eq!(THEME_DIR, "theme");
        assert_eq!(LANG_DIR, "lang");
        assert!(
            ConfigFile::ALL
                .iter()
                .all(|f| !f.file_name().starts_with(THEME_DIR)),
            "тема не должна быть файлом настроек"
        );
        // Выбор темы и языка — обычные настройки в общем файле.
        assert!(find(ConfigFile::Global, "General.theme").is_some());
        assert!(find(ConfigFile::Global, "General.language").is_some());
    }

    /// Режимы, названные в `docs/Ru/modes.md`.
    ///
    /// Секция конфига — это режим, и имя секции обязано быть именем режима.
    /// Список взят оттуда, а не выдуман: секция `[word]` годами стояла в схеме
    /// без единого основания, и по файлу было невозможно понять, что это.
    const MODES: &[&str] = &[
        // Режимы edit: type, view, replace, selection, search.
        "type",
        "view",
        "replace",
        "selection",
        "search",
        // Режимы files: browse, git, search, selection.
        "browse",
        "git",
    ];

    /// Секции, которые не режимы: у них своя причина быть.
    ///
    /// Экран статистики — не режим правки, а отдельная сущность со своими
    /// подсекциями, и в списке режимов его быть не должно.
    const NOT_MODES: &[&str] = &[
        "Savescreen",
        "Savescreen.Statistic",
        "Savescreen.Setting",
        "Savescreen.Setting.history_usage",
    ];

    #[test]
    fn every_section_is_a_mode_that_exists() {
        // Секция конфига называет режим. Режима, которого нет, быть не может:
        // иначе в конфиге появляется имя, которому ничто не соответствует, и
        // пользователь гадает, что оно значит.
        for key in KEYS {
            let section = key.section();
            if section == GENERAL || NOT_MODES.contains(&section) {
                continue;
            }
            assert!(
                MODES.contains(&section),
                "{}: секция `{section}` не режим из docs/Ru/modes.md: {:?}",
                key.path,
                MODES
            );
        }
    }

    #[test]
    fn every_section_name_is_readable() {
        // Имя секции читает человек в конфиге. Одно слово без понятного смысла
        // или выдумка — плохое имя, каким бы коротким оно ни было.
        for key in KEYS {
            let section = key.section();
            if section == GENERAL || NOT_MODES.contains(&section) {
                continue;
            }
            assert!(
                section.chars().all(|c| c.is_ascii_lowercase()),
                "{}: секция `{section}` должна быть одним словом латиницей",
                key.path
            );
            assert!(
                section.len() >= 2,
                "{}: секция `{section}` короче двух букв — однобуквенные оставлены под алиасы",
                key.path
            );
        }
    }

    #[test]
    fn a_mode_section_is_never_nested_into_itself() {
        // Третий уровень был заведён ради секции `[word.word]` и остался бы
        // навсегда, если бы его не убрали. Внутри режима режимов не бывает.
        for key in KEYS {
            let parts: Vec<&str> = key.path.split('.').collect();
            let limit = if NOT_MODES.contains(&key.section()) {
                4
            } else {
                2
            };
            assert!(
                parts.len() <= limit,
                "{}: лишний уровень вложенности: {}",
                key.path,
                key.path
            );
        }
    }

    #[test]
    fn every_key_has_a_section() {
        // Ключ без секции некуда положить: всё группируется по режимам.
        for key in KEYS {
            assert!(!key.section().is_empty(), "ключ {} без секции", key.path);
        }
    }

    #[test]
    fn state_name_is_not_in_the_key() {
        // Состояние названо файлом, дублировать его в ключе незачем.
        for key in KEYS {
            assert!(
                !key.path.starts_with("Edit.")
                    && !key.path.starts_with("Command.")
                    && !key.path.starts_with("Files."),
                "ключ {} повторяет имя состояния",
                key.path
            );
        }
    }

    #[test]
    fn third_level_allowed_only_inside_a_mode() {
        // В `[General]` режимов не бывает, вложенность там лишняя.
        for key in KEYS {
            let depth = key.section().split('.').count();
            if NOT_MODES.contains(&key.section()) {
                assert!(depth <= 3, "ключ {} слишком глубок", key.path);
            } else if key.is_general() {
                assert_eq!(depth, 1, "ключ {} лишней вложенности в General", key.path);
            } else {
                assert!(depth <= 2, "ключ {} слишком глубок для режима", key.path);
            }
        }
    }

    #[test]
    fn keys_are_unique_within_a_file() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert!(
                    a.file != b.file || a.path != b.path,
                    "ключ {} повторяется в {:?}",
                    a.path,
                    a.file
                );
            }
        }
    }

    #[test]
    fn keys_have_no_abbreviations() {
        // Сокращения недопустимы: имя читает человек в конфиге. Проверяем
        // слова целиком, а не подстроки: `history` — обычное слово.
        // Критерий не длина слова, а непрозрачность: короткие обычные слова
        // вроде `on` или `in` читаются, а `tmo` приходится расшифровывать.
        let banned = ["tmo", "isz", "cnt", "maxlen", "cfg", "amt", "tmp", "cur"];
        for key in KEYS {
            for word in key.key().split('_') {
                assert!(
                    !banned.contains(&word),
                    "ключ {} содержит сокращение {word}",
                    key.path
                );
            }
        }
    }

    #[test]
    fn every_key_has_a_comment() {
        for key in KEYS {
            assert!(!key.comment.is_empty(), "ключ {} без комментария", key.path);
        }
    }

    #[test]
    fn defaults_match_their_kind() {
        for key in KEYS {
            match key.kind {
                Kind::Bool => assert!(
                    key.default == "true" || key.default == "false",
                    "{}: значение по умолчанию не да/нет",
                    key.path
                ),
                Kind::U16 => assert!(
                    key.default.parse::<u16>().is_ok(),
                    "{}: значение по умолчанию не помещается в два байта",
                    key.path
                ),
                Kind::U8 => assert!(
                    key.default.parse::<u8>().is_ok(),
                    "{}: значение по умолчанию не помещается в байт",
                    key.path
                ),
                Kind::Str | Kind::Path | Kind::Array | Kind::Int | Kind::Float => {}
            }
        }
    }

    #[test]
    fn lookup_splits_section_and_key() {
        let key = find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(key.section(), "General");
        assert_eq!(key.key(), "history_depth");
        assert!(key.is_general());
        assert!(find_in(ConfigFile::Edit, "General", "history_depth").is_some());
        // В другом файле такого ключа нет.
        assert!(find_in(ConfigFile::Files, "General", "history_depth").is_none());
    }
}

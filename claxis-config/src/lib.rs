//! Конфиг: набор файлов, по одному на состояние.
//!
//! Крейт ничего не знает про состояния редактора как таковые: он знает
//! список ключей из [`schema`]. Группировка по состояниям — часть схемы, а не
//! самого конфига.
//!
//! Подписи и комментарии приходят из `claxis-i18n`: конфиг зависит от
//! локализации, но не наоборот.

pub mod error;
pub mod generate;
pub mod load;
pub mod messages;
pub mod path;
pub mod schema;
pub mod watch;

pub use error::{Error, Result};
pub use load::{Issue, Loaded, Problem};
pub use messages::{En, Messages};
pub use schema::TableDef;
pub use schema::{ConfigFile, GENERAL, KEYS, KeyDef, Kind};
pub use watch::Watch;

use toml_edit::Item;

/// Конфиг целиком: по файлу на состояние.
///
/// Каждое состояние открыто отдельно: так комментарии и порядок ключей внутри
/// файла не зависят от того, что происходит в соседнем.
#[derive(Clone, Debug, Default)]
pub struct Config {
    files: Vec<(ConfigFile, toml_edit::DocumentMut)>,
}

impl Config {
    /// Пустой конфиг: ни одного файла.
    pub fn new() -> Self {
        Self::default()
    }

    /// Разобранные файлы состояний.
    pub fn from_documents(
        docs: impl IntoIterator<Item = (ConfigFile, toml_edit::DocumentMut)>,
    ) -> Self {
        Self {
            files: docs.into_iter().collect(),
        }
    }

    /// Документ файла, если он открыт.
    pub fn doc(&self, file: ConfigFile) -> Option<&toml_edit::DocumentMut> {
        self.files.iter().find(|(f, _)| *f == file).map(|(_, d)| d)
    }

    /// Имена открытых файлов.
    pub fn open_files(&self) -> Vec<ConfigFile> {
        self.files.iter().map(|(f, _)| *f).collect()
    }

    /// Прочитать ключ.
    ///
    /// `None`, если файла нет или ключ в нём не задан: значит, работает
    /// значение по умолчанию из схемы.
    pub fn get(&self, def: &'static KeyDef) -> Option<&toml_edit::Item> {
        let table = self.doc(def.file)?;
        let mut current = table.as_table();
        for part in def.section().split('.') {
            current = current.get(part)?.as_table()?;
        }
        current.get(def.key())
    }

    /// Строковое значение ключа из конфига, иначе значение по умолчанию.
    pub fn string_or_default(&self, def: &'static KeyDef) -> String {
        self.get(def)
            .and_then(|i| i.as_str())
            .unwrap_or(def.default)
            .to_string()
    }

    /// Значение от 0 до 65535 из конфига, иначе значение по умолчанию.
    ///
    /// Значение по умолчанию берётся из схемы и обязано быть числом: если оно
    /// не такое, это огрех схемы, и о нём сообщается, а не подставляется
    /// тихий ноль.
    pub fn u16_or_default(&self, def: &'static KeyDef) -> crate::Result<u16> {
        match self.get(def).and_then(Item::as_integer) {
            Some(n) => u16::try_from(n).map_err(|_| crate::Error::BadValue {
                key: def.path.to_string(),
                got: n.to_string(),
                expected: "0..65535",
            }),
            None => def
                .default
                .parse()
                .map_err(|_| crate::Error::BadSchemaDefault {
                    key: def.path,
                    value: def.default,
                    expected: "a number",
                }),
        }
    }

    /// Значение от 0 до 255 из конфига, иначе значение по умолчанию.
    pub fn u8_or_default(&self, def: &'static KeyDef) -> crate::Result<u8> {
        match self.get(def).and_then(Item::as_integer) {
            Some(n) => u8::try_from(n).map_err(|_| crate::Error::BadValue {
                key: def.path.to_string(),
                got: n.to_string(),
                expected: "0..255",
            }),
            None => def
                .default
                .parse()
                .map_err(|_| crate::Error::BadSchemaDefault {
                    key: def.path,
                    value: def.default,
                    expected: "a number",
                }),
        }
    }

    /// Логическое значение ключа из конфига, иначе значение по умолчанию.
    pub fn bool_or_default(&self, def: &'static KeyDef) -> crate::Result<bool> {
        match self.get(def).and_then(Item::as_bool) {
            Some(v) => Ok(v),
            None => match def.default {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(crate::Error::BadSchemaDefault {
                    key: def.path,
                    value: other,
                    expected: "true or false",
                }),
            },
        }
    }

    /// Значение табличной настройки: пары «ключ, значение» как они записаны.
    ///
    /// Пусто, если настройки нет или она не разобралась.
    pub fn table(&self, def: &'static TableDef) -> Vec<(String, String)> {
        let Some(document) = self.doc(def.file) else {
            return Vec::new();
        };
        let mut cursor: &dyn toml_edit::TableLike = document.as_table();
        for part in def
            .path
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or("")
            .split('.')
        {
            match cursor.get(part).and_then(|next| next.as_table_like()) {
                Some(table) => cursor = table,
                None => return Vec::new(),
            }
        }
        let Some(value) = cursor
            .get(def.path.rsplit('.').next().unwrap_or(""))
            .and_then(|v| v.as_table_like())
        else {
            return Vec::new();
        };
        value
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string().trim().to_string()))
            .collect()
    }

    /// Ключ есть в файле и не отключён плохой секцией.
    pub fn has(&self, def: &'static KeyDef) -> bool {
        self.get(def).is_some()
    }

    /// Ключи, которые встречаются в файле, но в схеме не значатся.
    ///
    /// Молчаливый пропуск означал бы, что опечатку в настройке никто не
    /// заметит, а редактор поведёт себя не так, как ожидают.
    pub fn unknown_keys(&self, file: ConfigFile) -> Vec<String> {
        let Some(doc) = self.doc(file) else {
            return Vec::new();
        };
        let mut unknown = Vec::new();
        for (section, item) in doc.as_table().iter() {
            let Some(table) = item.as_table() else {
                unknown.push(section.to_string());
                continue;
            };
            for (key, _) in table.iter() {
                if find(file, &format!("{section}.{key}")).is_none() {
                    unknown.push(format!("{section}.{key}"));
                }
            }
        }
        unknown
    }
}

/// Найти описание ключа по файлу и пути внутри него.
fn find(file: ConfigFile, path: &str) -> Option<&'static KeyDef> {
    schema::find(file, path)
}

/// Ключи одного файла, по порядку объявления в схеме.
pub fn keys_of(file: ConfigFile) -> impl Iterator<Item = &'static KeyDef> {
    schema::KEYS.iter().filter(move |k| k.file == file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_config(text: &str) -> Config {
        Config::from_documents([(ConfigFile::Edit, text.parse().unwrap())])
    }

    #[test]
    fn reads_a_value_from_the_right_file() {
        let config = edit_config("[General]\nhistory_depth = 64\n");
        let def = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(config.u16_or_default(def).unwrap(), 64);
    }

    #[test]
    fn falls_back_to_the_schema_default() {
        // Не задано — работает то, что в схеме.
        let config = edit_config("[General]\n");
        let def = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(
            config.u16_or_default(def).unwrap(),
            def.default.parse().unwrap()
        );
    }

    #[test]
    fn files_do_not_leak_into_each_other() {
        // history_depth живёт в edit.toml и не должен читаться из files.toml.
        let config = Config::from_documents([(
            ConfigFile::Files,
            "[General]\nhistory_depth = 64\n".parse().unwrap(),
        )]);
        let def = schema::find(ConfigFile::Edit, "General.history_depth").unwrap();
        assert_eq!(config.u16_or_default(def).unwrap(), 8192);
    }

    #[test]
    fn unknown_key_is_reported() {
        let config = edit_config("[General]\nhistory_dept = 64\n");
        let unknown = config.unknown_keys(ConfigFile::Edit);
        assert_eq!(unknown, vec!["General.history_dept".to_string()]);
    }

    #[test]
    fn known_keys_are_not_reported() {
        let config = edit_config("[General]\nhistory_depth = 64\n");
        assert!(config.unknown_keys(ConfigFile::Edit).is_empty());
    }

    #[test]
    fn a_mode_section_is_read() {
        // Секция режима разбирается и остаётся в документе.
        let config = edit_config("[type]\nindent_size = 4\n");
        assert_eq!(config.open_files(), vec![ConfigFile::Edit]);
        assert!(config.doc(ConfigFile::Edit).is_some());
    }
}

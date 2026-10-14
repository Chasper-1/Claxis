//! Конфиг и перевод: язык, значения настроек, подписи.
//!
//! Связывает `claxis-config` с `claxis-i18n`. Конфиг знает, что пользователь
//! выбрал русский язык; локализация знает, как на этом языке звучат подписи и
//! комментарии. Вместе они дают то, что показывается: значения и текст рядом
//! с ними.
//!
//! Конфиг разнесён по файлам: `edit.toml`, `command.toml`, `files.toml`. Здесь
//! берутся нужные ключи и складываются в настройки редактора.

use std::path::Path;

use crate::api::config::path::{lang_dir, theme_dir};
use crate::api::config::schema;
use crate::api::config::{Config, ConfigFile};
use crate::api::i18n::load::load;
use crate::api::i18n::{Catalog, DEFAULT_LANGUAGE, Error as LangError};
use crate::api::theme::DEFAULT_THEME;

/// Путь настройки, значение которой — встроенная таблица подписей истории.
const HISTORY_USAGE_TABLE: &str = "Savescreen.Setting.history_usage";

/// Значение ключа из встроенной таблицы, иначе значение по умолчанию.
fn table_string(config: &Config, key: &str, path: &str) -> String {
    let Some(def) = schema::find_table(ConfigFile::Global, path) else {
        return String::new();
    };
    config
        .table(def)
        .into_iter()
        .find(|(k, _)| k == key)
        // Таблица отдаёт значение как оно в TOML, вместе с кавычками: подпись
        // нужна сама по себе, без них.
        .map(|(_, v)| v.trim_matches('"').to_string())
        .or_else(|| {
            def.default
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
        .unwrap_or_default()
}

/// Настройки редактора вместе с языком интерфейса.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Глубина истории: сколько правок живёт в памяти до снапшота.
    pub history_depth: u32,
    /// Сколько снапшотов одного файла держать в кеше.
    pub snapshots_keep: u8,
    /// Писать ли снапшоты на диск между запусками.
    pub snapshots_persist: bool,
    /// Через сколько секунд снимать снапшот. Ноль — на каждой правке.
    pub snapshots_interval: u16,
    /// Показывать ли экран статистики после сохранения.
    pub savescreen_include: bool,
    /// Как показывать: `fullscreen`, `message`, `window`.
    pub savescreen_mode: String,
    /// Показывать ли введённые символы.
    pub savescreen_input_char: bool,
    /// Показывать ли удалённые символы.
    pub savescreen_delete_char: bool,
    /// Показывать ли занятую историю.
    pub savescreen_history_usage: bool,
    /// Подпись занятой истории в процентах. Места: `{used}`, `{percent}`.
    pub savescreen_history_usage_percent: String,
    /// Подпись занятой истории числами. Места: `{used}`, `{remaining}`.
    pub savescreen_history_usage_number: String,
    /// Активный язык.
    pub language: String,
    /// Каталог перевода для этого языка.
    pub catalog: Catalog,
    /// Активная тема.
    pub theme: String,
}

impl Settings {
    /// Настройки по умолчанию: английский, значения из схемы.
    pub fn defaults() -> Self {
        let mut settings = Self {
            history_depth: 0,
            snapshots_keep: 0,
            snapshots_persist: false,
            snapshots_interval: 0,
            savescreen_include: false,
            savescreen_mode: String::new(),
            savescreen_input_char: false,
            savescreen_delete_char: false,
            savescreen_history_usage: false,
            savescreen_history_usage_percent: String::new(),
            savescreen_history_usage_number: String::new(),
            language: DEFAULT_LANGUAGE.to_string(),
            catalog: Catalog::new(),
            theme: DEFAULT_THEME.to_string(),
        };
        settings.apply_defaults_table();
        settings.apply_defaults();
        settings
    }

    /// Прочитать настройки из набора файлов конфига.
    ///
    /// Ключи берутся из своих файлов: язык и тема из `config.toml`, глубина
    /// истории из `edit.toml`, снапшоты из `files.toml`. Файлы не видят
    /// друг друга.
    ///
    /// Каталог перевода здесь **не** читается: для этого нужен путь, а путь
    /// знает только редактор. Язык запоминается, файл грузится отдельно —
    /// [`Settings::load_language`].
    pub fn from_config(config: &Config) -> Self {
        let mut settings = Self::defaults();

        if let Some(def) = schema::find(ConfigFile::Global, "General.language") {
            settings.language = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "General.theme") {
            settings.theme = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Edit, "General.history_depth") {
            // Конфиг ограничивает число 65535, буферу нужен u32: привести
            // шире, чем есть, ничего не теряется.
            settings.history_depth = config.u16_or_default(def) as u32;
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_keep") {
            settings.snapshots_keep = config.u8_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_persist") {
            settings.snapshots_persist = config.bool_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_interval") {
            settings.snapshots_interval = config.u16_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.mode") {
            settings.savescreen_mode = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.include") {
            settings.savescreen_include = config.bool_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.input_char") {
            settings.savescreen_input_char = config.bool_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.delete_char") {
            settings.savescreen_delete_char = config.bool_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.history_usage") {
            settings.savescreen_history_usage = config.bool_or_default(def);
        }
        settings.savescreen_history_usage_percent =
            table_string(config, "percent", HISTORY_USAGE_TABLE);
        settings.savescreen_history_usage_number =
            table_string(config, "number", HISTORY_USAGE_TABLE);
        settings
    }

    /// Загрузить файл перевода активного языка.
    ///
    /// Каталог кладётся рядом с языком, потому что они всегда меняются
    /// вместе: сменился код — сменился и набор строк.
    ///
    /// Ошибка возвращается, а не проглатывается: пользователь попросил
    /// интерфейс на языке, которого нет, и должен узнать об этом, а не
    /// получить английский вместо своего.
    pub fn load_language(&mut self, config_dir: &Path) -> Result<(), LangError> {
        self.catalog = load(lang_dir(config_dir), &self.language)?;
        Ok(())
    }

    /// Проверить, что выбранная тема есть.
    ///
    /// Содержимое темы пока не разбирается, поэтому проверяется только
    /// наличие: имя из конфига обязано чему-то соответствовать. Имя темы —
    /// это будущий путь, а путь не должен вылезать за пределы папки тем.
    pub fn check_theme(&self, config_dir: &Path) -> crate::api::theme::Result<()> {
        crate::api::theme::find(theme_dir(config_dir), &self.theme)?;
        Ok(())
    }

    /// Заполнить незаданные значения дефолтами из схемы.
    fn apply_defaults_table(&mut self) {
        if let Some(def) = schema::find_table(ConfigFile::Global, HISTORY_USAGE_TABLE) {
            for (key, value) in def.default {
                match *key {
                    "percent" => self.savescreen_history_usage_percent = value.to_string(),
                    "number" => self.savescreen_history_usage_number = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    fn apply_defaults(&mut self) {
        if let Some(def) = schema::find(ConfigFile::Edit, "General.history_depth") {
            self.history_depth = def.default.parse().unwrap_or(0);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_keep") {
            self.snapshots_keep = def.default.parse().unwrap_or(0);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_persist") {
            self.snapshots_persist = def.default == "true";
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_interval") {
            self.snapshots_interval = def.default.parse().unwrap_or(0);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.mode") {
            self.savescreen_mode = def.default.to_string();
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.include") {
            self.savescreen_include = def.default == "true";
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.input_char") {
            self.savescreen_input_char = def.default == "true";
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.delete_char") {
            self.savescreen_delete_char = def.default == "true";
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.history_usage") {
            self.savescreen_history_usage = def.default == "true";
        }
    }
}

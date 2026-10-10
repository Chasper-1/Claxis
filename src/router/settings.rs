//! Конфиг и перевод: язык, значения настроек, подписи.
//!
//! Связывает `claxis-config` с `claxis-i18n`. Конфиг знает, что пользователь
//! выбрал русский язык; локализация знает, как на этом языке звучат подписи и
//! комментарии. Вместе они дают то, что показывается: значения и текст рядом
//! с ними.
//!
//! Конфиг разнесён по файлам: `edit.toml`, `command.toml`, `files.toml`. Здесь
//! берутся нужные ключи и складываются в настройки редактора.

use claxis_config::Config;
use claxis_config::schema::{self, ConfigFile};
use claxis_i18n::Catalog;

/// Настройки редактора вместе с языком интерфейса.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Глубина истории: сколько правок живёт в памяти до снапшота.
    pub history_depth: u32,
    /// Сколько снапшотов одного файла держать в кеше.
    pub snapshots_keep: u32,
    /// Писать ли снапшоты на диск между запусками.
    pub snapshots_persist: bool,
    /// Активный язык.
    pub language: String,
    /// Каталог перевода для этого языка.
    pub catalog: Catalog,
}

impl Settings {
    /// Настройки по умолчанию: английский, значения из схемы.
    pub fn defaults() -> Self {
        let mut settings = Self {
            history_depth: 0,
            snapshots_keep: 0,
            snapshots_persist: false,
            language: claxis_i18n::path::default_language().to_string(),
            catalog: Catalog::new(),
        };
        settings.apply_defaults();
        settings
    }

    /// Прочитать настройки из набора файлов конфига.
    ///
    /// Ключи берутся из своих файлов: глубина истории из `edit.toml`,
    /// снапшоты из `files.toml`. Файлы не видят друг друга.
    pub fn from_config(config: &Config) -> Self {
        let mut settings = Self::defaults();

        if let Some(def) = schema::find(ConfigFile::Edit, "General.history_depth") {
            settings.history_depth = config.u32_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_keep") {
            settings.snapshots_keep = config.u32_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_persist") {
            settings.snapshots_persist = config.bool_or_default(def);
        }
        settings
    }

    /// Заполнить незаданные значения дефолтами из схемы.
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
    }
}

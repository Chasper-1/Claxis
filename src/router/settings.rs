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

/// Значение ключа из встроенной таблицы подписей истории.
///
/// Таблица приходит в конфиг тем же путём, что и обычный ключ. Если её нет в
/// конфиге, работает `apply_defaults_table`, который берёт то же значение из
/// схемы: два места читают одну таблицу, и двойной поиск здесь не нужен.
fn table_string(config: &Config, key: &str, path: &str) -> Option<String> {
    let def = schema::find_table(ConfigFile::Global, path)?;
    config
        .table(def)
        .into_iter()
        .find(|(k, _)| k == key)
        // Таблица отдаёт значение как оно в TOML, вместе с кавычками: подпись
        // нужна сама по себе, без них.
        .map(|(_, v)| v.trim_matches('"').to_string())
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
    ///
    /// Ошибка значит, что умолчание схемы не разбирается: это огрех схемы, а
    /// не пользователя, но тихий ноль здесь недопустим.
    pub fn defaults() -> Result<Self, crate::api::config::Error> {
        Self::from_config(&Config::new())
    }

    /// Пустые настройки: все поля заполняет [`Settings::from_config`].
    fn empty() -> Self {
        Self {
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
        }
    }

    /// Прочитать настройки из набора файлов конфига.
    ///
    /// Ключи берутся из своих файлов: язык и тема из `config.toml`, глубина
    /// истории из `edit.toml`, снапшоты из `files.toml`. Файлы не видят
    /// друг друга.
    ///
    /// Ошибка приходит от схемы, а не от пользователя: значение по умолчанию
    /// в схеме обязано соответствовать своему виду. Молчаливый ноль означал бы,
    /// что редактор работает с настройкой, которой никто не писал.
    ///
    /// Каталог перевода здесь **не** читается: для этого нужен путь, а путь
    /// знает только редактор. Язык запоминается, файл грузится отдельно —
    /// [`Settings::load_language`].
    pub fn from_config(config: &Config) -> Result<Self, crate::api::config::Error> {
        let mut settings = Self::empty();
        settings.apply_defaults_table();

        if let Some(def) = schema::find(ConfigFile::Global, "General.language") {
            settings.language = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "General.theme") {
            settings.theme = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Edit, "General.history_depth") {
            // Конфиг ограничивает число 65535, буферу нужен u32: привести
            // шире, чем есть, ничего не теряется.
            settings.history_depth = config.u16_or_default(def)? as u32;
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_keep") {
            settings.snapshots_keep = config.u8_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_persist") {
            settings.snapshots_persist = config.bool_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Files, "General.snapshots_interval") {
            settings.snapshots_interval = config.u16_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.mode") {
            settings.savescreen_mode = config.string_or_default(def);
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.include") {
            settings.savescreen_include = config.bool_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.input_char") {
            settings.savescreen_input_char = config.bool_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.delete_char") {
            settings.savescreen_delete_char = config.bool_or_default(def)?;
        }
        if let Some(def) = schema::find(ConfigFile::Global, "Savescreen.Statistic.history_usage") {
            settings.savescreen_history_usage = config.bool_or_default(def)?;
        }
        // Таблицы может не быть: тогда остаётся значение из схемы, а не
        // пустая подпись.
        if let Some(value) = table_string(config, "percent", HISTORY_USAGE_TABLE) {
            settings.savescreen_history_usage_percent = value;
        }
        if let Some(value) = table_string(config, "number", HISTORY_USAGE_TABLE) {
            settings.savescreen_history_usage_number = value;
        }
        Ok(settings)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каталог под тест, удаляемый вместе с собой.
    struct Temp {
        dir: std::path::PathBuf,
        store: std::path::PathBuf,
    }

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-settings-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let store = dir.join("store");
            std::fs::create_dir_all(&store).unwrap();
            Self { dir, store }
        }

        /// Каталог хранилища рядом с конфигом, чтобы тесты не трогали кеш
        /// пользователя.
        fn store_dir(&self) -> &std::path::Path {
            &self.store
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn defaults_come_from_the_schema() {
        // Настройки без конфига берутся из схемы, а не из зашитых чисел.
        let settings = Settings::defaults().unwrap();
        let depth = crate::api::config::schema::find(
            crate::api::config::ConfigFile::Edit,
            "General.history_depth",
        )
        .unwrap()
        .default
        .parse::<u32>()
        .unwrap();
        assert_eq!(settings.history_depth, depth);
        // Схема конфига и умолчание буфера — два разных источника одного
        // значения. Разойтись они не должны: иначе подключённый буфер и
        // сгенерированный конфиг говорили бы разное.
        assert_eq!(
            crate::api::buffer::defaults::buffer::HISTORY_DEPTH.value,
            depth,
            "умолчание буфера и умолчание схемы конфига обязаны совпадать"
        );
        assert_eq!(settings.snapshots_keep, 3);
        assert!(settings.snapshots_persist);
        assert_eq!(settings.language, "en");
    }

    #[test]
    fn each_file_owns_its_keys() {
        // Глубина истории живёт в edit.toml, снапшоты — в files.toml.
        let config = crate::api::config::Config::from_documents([
            (
                crate::api::config::ConfigFile::Edit,
                "[General]\nhistory_depth = 64\n".parse().unwrap(),
            ),
            (
                crate::api::config::ConfigFile::Files,
                "[General]\nsnapshots_keep = 7\nsnapshots_persist = false\n"
                    .parse()
                    .unwrap(),
            ),
        ]);

        let settings = Settings::from_config(&config).unwrap();
        assert_eq!(settings.history_depth, 64);
        assert_eq!(settings.snapshots_keep, 7);
        assert!(!settings.snapshots_persist);
    }

    #[test]
    fn a_missing_history_table_keeps_the_schema_default() {
        // Нет таблицы в конфиге — значит, работает умолчание схемы, а не
        // пустая подпись: иначе экран статистики показал бы пустое место.
        let config = crate::api::config::Config::from_documents([(
            crate::api::config::ConfigFile::Global,
            "[General]\n".parse().unwrap(),
        )]);
        let settings = Settings::from_config(&config).unwrap();
        assert_eq!(settings.savescreen_history_usage_percent, "{used}");
        assert_eq!(settings.savescreen_history_usage_number, "{remaining}");
    }

    #[test]
    fn a_file_does_not_affect_another() {
        // Ключ из чужого файла игнорируется, а не применяется как будто свой.
        let config = crate::api::config::Config::from_documents([(
            crate::api::config::ConfigFile::Files,
            "[General]\nhistory_depth = 64\n".parse().unwrap(),
        )]);
        let settings = Settings::from_config(&config).unwrap();
        assert_eq!(settings.history_depth, 8192);
    }

    #[test]
    fn every_file_opens_independently() {
        // Все файлы открываются вместе и не мешают друг другу: общий, три
        // состояния, темы и локализация.
        let config = crate::api::config::Config::from_documents(
            crate::api::config::ConfigFile::ALL
                .iter()
                .map(|f| (*f, "[General]\n".parse().unwrap())),
        );
        let open = config.open_files();
        assert_eq!(open.len(), crate::api::config::ConfigFile::ALL.len());
        for f in crate::api::config::ConfigFile::ALL {
            assert!(open.contains(f), "файл {} не открыт", f.file_name());
        }
    }

    #[test]
    fn common_settings_are_read_from_the_editor_file() {
        // Общие настройки редактора лежат в config.toml, а не в состоянии.
        let config = crate::api::config::Config::from_documents([(
            crate::api::config::ConfigFile::Global,
            "[General]\nhold = { alt = 350, ctrl = 200 }\n"
                .parse()
                .unwrap(),
        )]);
        let table = crate::api::config::schema::find_table(
            crate::api::config::ConfigFile::Global,
            "General.hold",
        )
        .unwrap();
        let pairs = config.table(table);
        assert_eq!(
            pairs,
            vec![
                ("alt".to_string(), "350".to_string()),
                ("ctrl".to_string(), "200".to_string())
            ]
        );

        // Тот же ключ в файле состояния ничего не делает.
        let leaked = crate::api::config::Config::from_documents([(
            crate::api::config::ConfigFile::Edit,
            "[General]\nhold = { alt = 350, ctrl = 200 }\n"
                .parse()
                .unwrap(),
        )]);
        assert!(leaked.table(table).is_empty());
    }

    #[test]
    fn unknown_key_is_reported_per_file() {
        // Опечатку должно быть видно, и видно в том файле, где она.
        let config = crate::api::config::Config::from_documents([(
            crate::api::config::ConfigFile::Edit,
            "[General]\nhistory_dept = 64\n".parse().unwrap(),
        )]);
        assert_eq!(
            config.unknown_keys(crate::api::config::ConfigFile::Edit),
            vec!["General.history_dept".to_string()]
        );
        assert!(
            config
                .unknown_keys(crate::api::config::ConfigFile::Files)
                .is_empty()
        );
    }

    #[test]
    fn editor_builds_session_from_settings() {
        // Настройки действительно доходят до сессии, а не лежат мёртвым грузом.
        let tmp = Temp::new("editor");
        let mut settings = Settings::defaults().unwrap();
        settings.history_depth = 32;
        settings.snapshots_keep = 5;
        settings.snapshots_persist = false;

        let editor = crate::router::Editor::with_settings(tmp.store_dir(), settings).unwrap();
        assert_eq!(editor.session().history_depth(), 32);
        assert_eq!(editor.session().keep(), 5);
        assert!(!editor.session().persist());
        assert_eq!(editor.language(), "en");
    }

    #[test]
    fn the_statistics_screen_defaults_to_the_schema() {
        let settings = Settings::defaults().unwrap();
        assert!(settings.savescreen_include);
        assert_eq!(settings.savescreen_mode, "fullscreen");
        assert_eq!(settings.savescreen_history_usage_percent, "{used}");
        assert_eq!(settings.savescreen_history_usage_number, "{remaining}");
    }
}

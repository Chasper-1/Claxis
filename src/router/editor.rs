//! Всё вместе: то, с чего начинается запуск.
//!
//! Верхний уровень маршрутизатора. Держит терминал, настройки и сессию и
//! переводит настройки в параметры того, что их использует: глубина истории
//! уходит в буфер, `keep` и `persist` — в хранилище снапшотов.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api::config::schema::ConfigFile;
use crate::api::config::{Issue, generate};

use crate::api::i18n::Catalog;

use crate::router::{Applied, Session, SessionError, Settings};

/// Что не так с конфигом при запуске.
#[derive(Debug)]
pub enum StartError {
    /// Сессия не открылась.
    Session(SessionError),
    /// Объявления команд не разобрались: `{reason}`.
    ///
    /// Список команд приходит из крейтов при сборке программы, поэтому такая
    /// ошибка означает опечатку в объявлении, а не поломку у пользователя.
    Commands(crate::api::commands::Error),
    /// Каталог конфига недоступен: `{path}`, `{reason}`.
    ConfigDir {
        /// Путь к каталогу.
        path: String,
        /// Почему недоступен.
        reason: String,
    },
    /// Домашнего каталога нет: конфиг положить некуда.
    NoHomeDir,
    /// Конфиг сломан настолько, что продолжать не с чем.
    ///
    /// Так бывает, только если сломаны все файлы разом и запасной не
    /// сохранился: работать не с чем.
    NoUsableConfig {
        /// Сколько проблем в файлах.
        issues: usize,
    },
}

impl StartError {
    /// Текст на языке каталога. По умолчанию — английский.
    pub fn message(&self, m: &dyn crate::messages::Messages) -> String {
        match self {
            StartError::Session(e) => e.message(m),
            StartError::Commands(e) => m.commands_error(e),
            StartError::ConfigDir { path, reason } => m.config_dir_failed(path, reason),
            StartError::NoHomeDir => m.no_home_dir(),
            StartError::NoUsableConfig { issues } => m.no_usable_config(*issues),
        }
    }

    /// Текст на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for StartError {}

/// Редактор: собранное из всех крейтов.
pub struct Editor {
    session: Session,
    settings: Settings,
    /// Проблемы конфига: файл, строка, ключ, суть.
    pub issues: Vec<Issue>,
    /// Выбранной темы нет в папке тем.
    ///
    /// Отдельно от языка по той же причине: тема лежит не в конфиге, и её
    /// отсутствие не отключает секцию конфига. Редактор открывается с темой
    /// по умолчанию, а правка видна сразу.
    pub theme_error: Option<crate::api::theme::Error>,
    /// Файл перевода не прочитан или разобран неправильно.
    ///
    /// Отдельно от проблем конфига: язык лежит не в конфиге, и поломка в нём
    /// не отключает секцию конфига. Интерфейс остаётся английским, а правка
    /// видна сразу.
    pub language_error: Option<crate::api::i18n::Error>,
    /// В схеме конфига значение по умолчанию не того вида, чем объявлен ключ.
    ///
    /// Огрех схемы, а не конфига: пользователь ничего не писал. Сообщаем и
    /// работаем на значениях по умолчанию, чтобы редактор открылся.
    pub schema_error: Option<crate::api::config::Error>,
    /// Файлы, которых не было и которые созданы при первом запуске.
    pub created: Vec<PathBuf>,
    /// Редактор пошёл на последнем корректном конфиге.
    pub used_last_good: bool,
    /// Откуда читается конфиг.
    config_dir: PathBuf,
    /// Наблюдатель за каталогом конфига.
    ///
    /// `None`, если система не смогла подписаться: конфиг перестанет
    /// применяться сам, и об этом честно скажем, а не промолчим.
    pub watch: Option<crate::api::disk::Watch>,
    /// Папки, за которыми следить не вышло, с причинами.
    ///
    /// Правки внутри них не применятся сами: только перезапуск. Пользователю
    /// об этом сказано, иначе он ищет ошибку там, где её нет.
    pub blind_spots: Vec<String>,
    /// Поломки наблюдения за файлами, ждущие показа.
    watch_failures: Vec<String>,
    /// Команды, объявленные крейтами.
    ///
    /// Собирается один раз при запуске из объявлений: список команд живёт у
    /// крейтов, а редактор знает, какие имена существуют.
    pub commands: crate::api::commands::Registry,
}

/// Что вышло из перечитывания конфига.
#[derive(Clone, Debug, Default)]
pub struct Reloaded {
    /// Что применилось сразу, а что ждёт перезапуска.
    pub applied: Applied,
    /// Проблемы, если конфиг негоден.
    pub issues: Vec<Issue>,
    /// Конфиг принят.
    pub accepted: bool,
}

impl Reloaded {
    /// Конфиг негоден: показываем, где сломалось.
    pub fn is_ok(&self) -> bool {
        self.accepted
    }

    /// Первая проблема: на неё ставится курсор.
    pub fn first_issue(&self) -> Option<&Issue> {
        self.issues.first()
    }
}

impl Editor {
    /// Открыть редактор с конфигом пользователя и хранилищем в кеше.
    pub fn open() -> Result<Self, StartError> {
        let dir = crate::api::config::path::user_dir().ok_or(StartError::NoHomeDir)?;
        let store_dir = crate::api::disk::StorePaths::from_env().dir;
        Self::open_in(&dir, &store_dir)
    }

    /// Конфиг задан явно, хранилище — кеш пользователя.
    pub fn open_at(dir: &Path) -> Result<Self, StartError> {
        let store_dir = crate::api::disk::StorePaths::from_env().dir;
        Self::open_in(dir, &store_dir)
    }

    /// Конфиг и хранилище заданы явно — нужно в тестах.
    pub fn open_in(config_dir: &Path, store_dir: &Path) -> Result<Self, StartError> {
        // Порядок важен: сначала читается конфиг, потом под его настройки
        // создаётся сессия. Иначе настройки из файла просто никуда не денутся.
        let generated = generate::ensure_files(config_dir, &Catalog::new()).map_err(|e| {
            StartError::ConfigDir {
                path: config_dir.display().to_string(),
                reason: e.to_string(),
            }
        })?;

        let loaded = generate::read_all(config_dir);
        let issues = loaded.issues;

        let (config, used_last_good) = if issues.is_empty() {
            save_last_good(store_dir, config_dir);
            (loaded.config, false)
        } else {
            // Текущий конфиг не годен: работаем на последнем корректном.
            // Пустой редактор со сброшенными настройками хуже, чем рабочий со
            // старыми, поэтому падать здесь нельзя.
            match load_last_good(store_dir) {
                Some(previous) => (previous, true),
                None => {
                    return Err(StartError::NoUsableConfig {
                        issues: issues.len(),
                    });
                }
            }
        };

        // Подписка на каталог: конфиг могут поменять снаружи, и открытый
        // редактор обязан это увидеть. Наблюдение — дело крейта диска, здесь
        // только подписка на то, что этому редактору интересно.
        let watched_dirs = [
            crate::api::config::path::theme_dir(config_dir),
            crate::api::config::path::lang_dir(config_dir),
        ];
        let watch = crate::api::disk::Watch::new(config_dir, &watched_dirs).ok();

        // Папки, за которыми следить не вышло: смена темы или языка в них не
        // применится сама. Пользователь должен знать про это, иначе он будет
        // искать ошибку в конфиге, которого никто не прочитал.
        // Папка без причины выглядит как «что-то сломалось в системе», а с
        // причиной человек сразу видит, куда смотреть.
        let blind_spots: Vec<String> = watch
            .as_ref()
            .map(|w| {
                w.blind_spots()
                    .iter()
                    .map(|b| format!("{}: {}", b.dir.display(), b.reason))
                    .collect()
            })
            .unwrap_or_default();

        // Ошибка схемы — не пользовательская: это огрех наших же значений по
        // умолчанию. Сообщаем и работаем на значениях по умолчанию, чтобы
        // редактор открылся: чинить схему должен тот, кто её писал.
        let (mut settings, schema_error) = match Settings::from_config(&config) {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::defaults(), Some(e)),
        };
        // Перевод не входит в конфиг, но отсутствие файла — это ровно та же
        // поломка: пользователь попросил язык, которого нет. Сообщаем, но не
        // отказываем запускаться: редактор с английским лучше, чем ошибка
        // вместо редактора.
        let language_error = settings.load_language(config_dir).err();
        let theme_error = settings.check_theme(config_dir).err();
        let mut session = Session::open_at_with(
            store_dir,
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )
        .map_err(StartError::Session)?;
        session.set_snapshot_interval(Duration::from_secs(u64::from(settings.snapshots_interval)));

        Ok(Self {
            settings,
            session,
            issues,
            theme_error,
            language_error,
            schema_error,
            created: generated.created,
            used_last_good,
            config_dir: config_dir.to_path_buf(),
            watch,
            blind_spots,
            watch_failures: Vec::new(),
            commands: super::commands::registry().map_err(StartError::Commands)?,
        })
    }

    /// Собрать редактор с готовыми настройками, без чтения файлов.
    pub fn with_settings(dir: impl AsRef<Path>, settings: Settings) -> Result<Self, SessionError> {
        let mut session = Session::open_at_with(
            dir.as_ref(),
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )?;
        session.set_snapshot_interval(Duration::from_secs(u64::from(settings.snapshots_interval)));
        Ok(Self {
            session,
            settings,
            issues: Vec::new(),
            theme_error: None,
            language_error: None,
            schema_error: None,
            created: Vec::new(),
            used_last_good: false,
            config_dir: dir.as_ref().to_path_buf(),
            watch: None,
            blind_spots: Vec::new(),
            watch_failures: Vec::new(),
            commands: super::commands::registry().map_err(SessionError::Commands)?,
        })
    }

    /// Сессия: документы и снапшоты.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Настройки, из которых собрано.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Активный язык.
    pub fn language(&self) -> &str {
        &self.settings.language
    }

    /// Каталог перевода активного языка.
    pub fn catalog(&self) -> &Catalog {
        &self.settings.catalog
    }

    /// Первая проблема конфига: на неё ставится курсор.
    pub fn first_issue(&self) -> Option<&Issue> {
        self.issues.first()
    }

    /// Перечитать конфиг после сохранения и применить, если он годен.
    ///
    /// Это и есть динамический конфиг: файл сохранили — редактор сразу
    /// подхватил изменения. Если конфиг сломан, он **не применяется**: в
    /// ответе останутся старые настройки и список проблем с файлом и
    /// строкой, чтобы подсветить место поломки.
    ///
    /// Ничего не дописывается и не дополняется: конфиг выглядит ровно так,
    /// как его написал пользователь.
    pub fn reload(&mut self) -> Reloaded {
        let loaded = generate::read_all(&self.config_dir);
        if !loaded.issues.is_empty() {
            // Сломанный конфиг не трогаем: пользователь должен увидеть, что
            // его правка не сработала, и где именно ошибка.
            return Reloaded {
                applied: Applied::default(),
                issues: loaded.issues,
                accepted: false,
            };
        }

        // Как и при старте: огрех схемы не должен ронять редактор, но и молчать о
        // нём нельзя.
        let (mut settings, schema_error) = match Settings::from_config(&loaded.config) {
            Ok(settings) => (settings, None),
            Err(e) => (Settings::defaults(), Some(e)),
        };
        // Перевод грузится здесь же: язык меняется в конфиге, и ждать
        // перезапуска для смены языка незачем.
        //
        // Файл перевода может быть поломан, а конфиг — в порядке. Тогда
        // конфиг применяется целиком, а интерфейс остаётся английским, и об
        // этом честно сообщаем.
        let language_error = settings.load_language(&self.config_dir).err();
        let theme_error = settings.check_theme(&self.config_dir).err();
        self.session
            .set_snapshot_interval(Duration::from_secs(u64::from(settings.snapshots_interval)));
        let applied = self.session.apply(&settings);
        self.settings = settings;
        self.issues = Vec::new();
        self.language_error = language_error;
        self.theme_error = theme_error;
        self.schema_error = schema_error;

        Reloaded {
            applied,
            issues: Vec::new(),
            accepted: true,
        }
    }

    /// Каталог, из которого читается конфиг.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Перечитать конфиг, если система сообщила, что файл изменился.
    ///
    /// Вызывается из цикла редактора. Возвращает `None`, если изменений нет,
    /// иначе результат применения.
    ///
    /// Наблюдение умеет само сломаться — например, переполнилась очередь
    /// событий. Молчать об этом нельзя: правки перестали применяться, а
    /// выглядит всё так же, поэтому такое возвращается проблемой, а человек
    /// видит сообщение.
    pub fn poll_watch(&mut self) -> Option<Reloaded> {
        let mut changed = self.watch.as_mut()?.changed();
        if changed.failures.is_empty() && changed.paths.is_empty() {
            return None;
        }
        // Наблюдение умеет само сломаться, и это не проблема конфига: файлы
        // могут быть в порядке, а применяться перестанут. Такое лежит
        // отдельно, чтобы не выдавать за ошибку в чужом файле.
        self.watch_failures.append(&mut changed.failures);

        // Крейт диска сообщает обо всём под корнем, а чьи это файлы — знает
        // редактор: чужая заметка рядом с конфигом не должна вызывать
        // перечитывание.
        let mine = self.config_files_changed(&changed.paths);
        if mine.is_empty() {
            return None;
        }
        Some(self.reload())
    }

    /// Какие из изменившихся путей относятся к конфигу.
    ///
    /// Свои — файлы настроек и всё, что лежит в папках тем и переводов: там
    /// каждая новая тема или новый язык появляются сами собой. Остальное в
    /// каталоге конфига к делу не относится.
    fn config_files_changed(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let theme = crate::api::config::path::theme_dir(&self.config_dir);
        let lang = crate::api::config::path::lang_dir(&self.config_dir);
        let own_file = |path: &PathBuf| {
            ConfigFile::ALL
                .iter()
                .any(|f| *path == self.config_dir.join(f.file_name()))
        };
        paths
            .iter()
            .filter(|path| own_file(path) || path.starts_with(&theme) || path.starts_with(&lang))
            .cloned()
            .collect()
    }

    /// Что сломалось в наблюдении за файлами.
    ///
    /// Наблюдение может молча перестать работать: правки перестают
    /// применяться, а выглядит всё как обычно. Такое пользователю
    /// показывается, иначе он будет искать ошибку не там.
    pub fn watch_failures(&self) -> &[String] {
        &self.watch_failures
    }

    /// Забрать накопленные поломки наблюдения: показаны один раз.
    pub fn take_watch_failures(&mut self) -> Vec<String> {
        std::mem::take(&mut self.watch_failures)
    }

    /// Следит ли редактор за каталогом конфига.
    pub fn is_watching(&self) -> bool {
        self.watch.is_some()
    }
}

/// Сохранить текущие файлы как последний корректный конфиг.
///
/// Кеш может быть недоступен — тогда просто не сохраняем: конфиг уже загружен,
/// и работа не должна вставать из-за кеша.
fn save_last_good(store_dir: &Path, dir: &Path) {
    let Ok(mut store) =
        crate::api::disk::SnapshotStore::open(&crate::api::disk::StorePaths::new(store_dir), 1)
    else {
        return;
    };
    for &file in ConfigFile::ALL {
        let path = dir.join(file.file_name());
        if let Ok(body) = std::fs::read_to_string(&path) {
            let _ = store.save_last_good(file.file_name(), &body);
        }
    }
}

/// Забрать последний корректный конфиг из кеша.
fn load_last_good(store_dir: &Path) -> Option<crate::api::config::Config> {
    let store =
        crate::api::disk::SnapshotStore::open(&crate::api::disk::StorePaths::new(store_dir), 1)
            .ok()?;
    let mut docs = Vec::new();
    for &file in ConfigFile::ALL {
        // Файла в кеше может не быть: он добавился позже, или кеш чистили.
        // Отсутствие одного файла не повод отказывать целиком — редактор с
        // частью настроек лучше, чем редактор, который не запустился.
        let Ok(Some(body)) = store.last_good(file.file_name()) else {
            continue;
        };
        // Разбор идёт по тем же правилам: запасной тоже может быть старым.
        if let Ok(doc) = body.parse::<toml_edit::DocumentMut>() {
            docs.push((file, doc));
        }
    }
    if docs.is_empty() {
        return None;
    }
    Some(crate::api::config::Config::from_documents(docs))
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
            let dir = std::env::temp_dir().join(format!("claxis-editor-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let store = dir.join("store");
            std::fs::create_dir_all(&store).unwrap();
            Self { dir, store }
        }

        /// Каталог конфига под тест.
        fn dir(&self) -> &std::path::Path {
            &self.dir
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

    /// Положить файл перевода в каталог конфига под временным тестом.
    fn write_translation(config_dir: &std::path::Path, lang: &str, body: &str) {
        let dir = config_dir.join("lang");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{lang}.toml")), body).unwrap();
    }

    /// Положить файл темы в каталог конфига под временным тестом.
    fn write_theme(config_dir: &std::path::Path, file: &str) {
        let dir = config_dir.join("theme");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), "").unwrap();
    }

    // ── Запуск с конфигом ───────────────────────────────────────────────────────

    #[test]
    fn first_run_creates_every_config_file() {
        // При первом запуске создаются все файлы разом, со всеми ключами.
        let tmp = Temp::new("first-run-cfg");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(!editor.created.is_empty(), "файлы должны быть созданы");
        for file in crate::api::config::ConfigFile::ALL {
            assert!(
                tmp.dir().join(file.file_name()).exists(),
                "{} не создан",
                file.file_name()
            );
        }
        assert!(editor.issues.is_empty(), "проблемы: {:?}", editor.issues);
        assert!(!editor.used_last_good);
    }

    #[test]
    fn generated_files_carry_every_key_with_a_comment() {
        let tmp = Temp::new("keys-with-comments");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        for key in crate::api::config::KEYS {
            let path = tmp.dir().join(key.file.file_name());
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains(key.key()),
                "{}: нет ключа {}",
                key.file.file_name(),
                key.key()
            );
            assert!(
                text.contains(key.comment),
                "{}: нет комментария к {}",
                key.file.file_name(),
                key.key()
            );
        }
    }

    #[test]
    fn settings_from_the_file_reach_the_session() {
        let tmp = Temp::new("settings-reach");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        std::fs::write(
            tmp.dir().join("edit.toml"),
            "[General]\nhistory_depth = 64\n",
        )
        .unwrap();
        std::fs::write(
            tmp.dir().join("files.toml"),
            "[General]\nsnapshots_keep = 7\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.session().history_depth(), 64);
        assert_eq!(editor.session().keep(), 7);
    }

    #[test]
    fn broken_config_falls_back_to_the_saved_one() {
        // Главное: сломанный конфиг не должен оставлять пользователя с пустым
        // редактором. Работаем на последнем корректном.
        let tmp = Temp::new("fallback");
        std::fs::create_dir_all(tmp.dir()).unwrap();

        // Первый запуск: всё в порядке, конфиг сохраняется.
        {
            let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
            assert!(editor.issues.is_empty());
            assert!(!editor.used_last_good);
        }

        // Второй запуск: пользователь сломал файл.
        std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 1\n").unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        assert!(editor.used_last_good, "должен был пойти на запасной конфиг");
        assert_eq!(editor.issues.len(), 1);
        let issue = editor.first_issue().unwrap();
        assert_eq!(issue.file, "edit.toml");
        assert_eq!(issue.line, 2);
        // Настройки те же, что и до поломки.
        assert_eq!(editor.session().history_depth(), 8192);
    }

    #[test]
    fn broken_config_on_first_run_has_nothing_to_fall_back_to() {
        // Падать можно только когда работать не с чем.
        let tmp = Temp::new("no-fallback");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        // Готовим сломанный файл ДО первого запуска: сохранять нечего.
        std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 1\n").unwrap();

        match Editor::open_in(tmp.dir(), tmp.store_dir()) {
            Ok(editor) => {
                // Запасной мог появиться из других файлов — тогда всё в порядке.
                assert!(editor.used_last_good || !editor.issues.is_empty());
            }
            Err(crate::router::StartError::NoUsableConfig { .. }) => {}
            Err(other) => panic!("не та ошибка: {other}"),
        }
    }

    #[test]
    fn editing_one_key_keeps_the_rest_of_the_file() {
        // Точечная запись не переписывает файл: комментарии и порядок целы.
        let tmp = Temp::new("point-write");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        let key = crate::api::config::schema::find(
            crate::api::config::ConfigFile::Edit,
            "General.history_depth",
        )
        .unwrap();
        crate::api::config::generate::write_value(tmp.dir(), key, "256").unwrap();

        let text = std::fs::read_to_string(tmp.dir().join("edit.toml")).unwrap();
        assert!(text.contains("history_depth = 256"), "{text}");
        assert!(text.contains(key.comment), "комментарий пропал: {text}");
        assert!(text.contains("0 to 65535"), "диапазон пропал: {text}");
    }

    // ── Динамический конфиг: сохранил — применилось ────────────────────────────

    #[test]
    fn saved_config_applies_without_restart() {
        // Главное: сохранение файла подхватывается сразу.
        let tmp = Temp::new("dynamic-apply");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        std::fs::write(
            tmp.dir().join("files.toml"),
            "[General]\nsnapshots_keep = 9\n",
        )
        .unwrap();

        let reloaded = editor.reload();
        assert!(
            reloaded.is_ok(),
            "конфиг должны были принять: {:?}",
            reloaded
        );
        assert!(reloaded.issues.is_empty());
        // Применилось сразу, без перезапуска.
        assert!(
            reloaded.applied.applied.contains(&"snapshots_keep"),
            "keep должен примениться сразу: {:?}",
            reloaded.applied
        );
        assert!(reloaded.applied.needs_restart.is_empty());
        assert_eq!(editor.session().keep(), 9);
    }

    #[test]
    fn broken_saved_config_is_refused_and_reported() {
        // Сломанный конфиг не применяется: старые настройки остаются, а где
        // ошибка — видно, чтобы подсветить.
        let tmp = Temp::new("dynamic-broken");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let before = editor.session().keep();

        std::fs::write(
            tmp.dir().join("files.toml"),
            "[General]\nsnapshots_keep = 9\nnonsense = 1\n",
        )
        .unwrap();

        let reloaded = editor.reload();
        assert!(!reloaded.is_ok(), "сломанный конфиг принят быть не должен");
        assert_eq!(reloaded.issues.len(), 1);

        let issue = reloaded.first_issue().expect("где сломалось");
        assert_eq!(issue.file, "files.toml");
        assert_eq!(issue.line, 3);
        assert_eq!(issue.key, "General.nonsense");

        // Настройки те же: сломанный конфиг ничего не поменял.
        assert_eq!(editor.session().keep(), before);
    }

    #[test]
    fn reload_writes_nothing_to_the_file() {
        // Никто ничего не дописывает: конфиг выглядит ровно так, как его написал
        // пользователь.
        let tmp = Temp::new("dynamic-no-write");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        let path = tmp.dir().join("files.toml");
        let mine = "[General]\nsnapshots_keep = 4\n";
        std::fs::write(&path, mine).unwrap();

        editor.reload();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            mine,
            "редактор не должен трогать файл пользователя"
        );
    }

    #[test]
    fn depth_change_asks_for_restart_instead_of_half_applying() {
        // Размер пула задан при создании буфера: на лету не меняется, и редактор
        // честно говорит об этом, а не делает вид.
        let tmp = Temp::new("dynamic-depth");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        std::fs::write(
            tmp.dir().join("edit.toml"),
            "[General]\nhistory_depth = 128\n",
        )
        .unwrap();

        let reloaded = editor.reload();
        assert!(reloaded.is_ok());
        assert!(
            reloaded.applied.needs_restart.contains(&"history_depth"),
            "глубина должна просить перезапуск: {:?}",
            reloaded.applied
        );
        // Буфер продолжает работать на прежней глубине: половинчатое применение
        // хуже, чем честная просьба перезапустить.
        assert_eq!(editor.session().opened_depth(), 8192);
    }

    #[test]
    fn reloading_an_unchanged_file_changes_nothing() {
        let tmp = Temp::new("dynamic-noop");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        let reloaded = editor.reload();
        assert!(reloaded.is_ok());
        assert!(reloaded.applied.applied.is_empty());
        assert!(reloaded.applied.needs_restart.is_empty());
    }

    // ── Наблюдение за каталогом конфига ─────────────────────────────────────────

    #[test]
    fn editor_watches_the_config_directory() {
        let tmp = Temp::new("watching");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.is_watching(), "редактор должен следить за конфигом");
    }

    #[test]
    fn external_edit_is_applied_without_touching_the_editor() {
        // Главное: правка снаружи видна открытому редактору.
        let tmp = Temp::new("external");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let before = editor.session().keep();

        // Пишем со стороны, как будто это другой редактор или git.
        std::fs::write(
            tmp.dir().join("files.toml"),
            "[General]\nsnapshots_keep = 6\n",
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut applied = None;
        while std::time::Instant::now() < deadline && applied.is_none() {
            if let Some(reloaded) = editor.poll_watch() {
                applied = Some(reloaded);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let reloaded = applied.expect("правка снаружи должна быть замечена");
        assert!(reloaded.is_ok(), "конфиг должны были принять: {reloaded:?}");
        assert!(before != editor.session().keep(), "настройка не сменилась");
        assert_eq!(editor.session().keep(), 6);
    }

    #[test]
    fn nothing_happens_when_the_config_is_quiet() {
        // Пока файлы не трогают, перечитывания быть не должно.
        let tmp = Temp::new("quiet");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        for _ in 0..10 {
            assert!(
                editor.poll_watch().is_none(),
                "перечитывание без изменения файла"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_broken_watcher_is_reported_and_not_shown_twice() {
        // Наблюдение может сломаться само: правки перестают применяться, а
        // выглядит всё как обычно. Молчать об этом нельзя, но и повторять одно
        // и то же сообщение на каждый кадр — тоже.
        let tmp = Temp::new("watch-failure");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        assert!(
            editor.watch_failures().is_empty(),
            "на свежем редакторе тихо"
        );

        // Подсовываем поломку так, как её приносит крейт диска.
        editor.watch_failures = vec!["очередь событий переполнилась".to_string()];
        editor.poll_watch();

        let shown = editor.take_watch_failures();
        assert_eq!(shown, vec!["очередь событий переполнилась".to_string()]);
        assert!(
            editor.watch_failures().is_empty(),
            "одна поломка — одно сообщение"
        );
        assert!(
            editor.take_watch_failures().is_empty(),
            "второй раз показывать нечего"
        );
    }

    #[test]
    fn a_file_that_is_our_business_is_filtered_out_before_the_config() {
        // Крейт диска сообщает обо всём под корнем, а чьи это файлы — знает
        // редактор: чужая правка рядом с конфигом не должна вызывать
        // перечитывание.
        let tmp = Temp::new("foreign-file");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

        std::fs::write(tmp.dir().join("заметки.txt"), "не конфиг").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if editor.poll_watch().is_some() {
                panic!("чужой файл не должен считаться изменением конфига");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn only_config_files_and_their_folders_count_as_ours() {
        // Проверка самого отбора: кто наш, а кто мимо.
        let tmp = Temp::new("which-files");
        std::fs::create_dir_all(tmp.dir()).unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let theme = crate::api::config::path::theme_dir(tmp.dir());
        let lang = crate::api::config::path::lang_dir(tmp.dir());

        let ours = vec![
            tmp.dir().join("config.toml"),
            tmp.dir().join("edit.toml"),
            tmp.dir().join("command.toml"),
            tmp.dir().join("files.toml"),
            theme.join("everforest.toml"),
            lang.join("ru.toml"),
        ];
        let mine = editor.config_files_changed(&ours);
        assert_eq!(mine.len(), ours.len(), "свои файлы отсеяны: {mine:?}");

        let foreign = vec![
            tmp.dir().join("заметки.txt"),
            tmp.dir().join(".config.toml.swp"),
            tmp.dir().join("просто-папка"),
        ];
        assert!(
            editor.config_files_changed(&foreign).is_empty(),
            "чужие приняты за наши"
        );
        assert!(
            editor.config_files_changed(&[]).is_empty(),
            "пустой список остаётся пустым"
        );
    }

    // ── Язык интерфейса ─────────────────────────────────────────────────────────

    #[test]
    fn a_language_from_the_config_reaches_the_interface() {
        // Главное: язык из config.toml действительно попадает в каталог, который
        // потом берут подписи. Иначе ключ был бы включён, но ничего не менял.
        let tmp = Temp::new("lang-from-config");
        write_translation(
            tmp.dir(),
            "ru",
            "\"Open file\" = \"Открыть файл\"\n\"Save file\" = \"Сохранить файл\"\n",
        );
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"ru\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "ru");
        assert!(
            editor.language_error.is_none(),
            "язык не загрузился: {:?}",
            editor.language_error
        );
        assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
    }

    #[test]
    fn english_needs_no_file_on_disk() {
        // Английский зашит в бинарник: папки lang может не быть вообще.
        let tmp = Temp::new("lang-english");
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "en");
        assert!(editor.language_error.is_none());
    }

    #[test]
    fn a_missing_language_file_is_reported_not_hidden() {
        // Пользователь попросил язык, которого нет. Молча отдать английский —
        // значит показать не тот интерфейс, который он выбрал.
        let tmp = Temp::new("lang-missing");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"ru\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "ru");
        let error = editor
            .language_error
            .as_ref()
            .expect("отсутствие файла должно быть сообщено");
        assert!(
            error.text().contains("ru.toml"),
            "в сообщении нет пути: {}",
            error.text()
        );
    }

    #[test]
    fn a_broken_translation_still_opens_the_editor() {
        // Поломка перевода — не повод не запускать редактор: конфиг в порядке,
        // работать есть над чем. Но о поломке сказано.
        let tmp = Temp::new("lang-broken");
        write_translation(tmp.dir(), "ru", "\"Open file\" = \n");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"ru\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.issues.is_empty(), "конфиг цел: {:?}", editor.issues);
        assert!(
            editor.language_error.is_some(),
            "поломка перевода не сообщена"
        );
        // Сессия работает: редактор открылся, а не отказался.
        assert_eq!(editor.session().keep(), 3);
    }

    #[test]
    fn switching_the_language_applies_without_restart() {
        // Язык меняется в конфиге и применяется сразу, как и остальные ключи.
        let tmp = Temp::new("lang-switch");
        write_translation(tmp.dir(), "ru", "\"Open file\" = \"Открыть файл\"\n");
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "en");

        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"ru\"\n",
        )
        .unwrap();
        let reloaded = editor.reload();
        assert!(reloaded.is_ok(), "{reloaded:?}");
        assert_eq!(editor.language(), "ru");
        assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
    }

    #[test]
    fn a_language_code_cannot_escape_the_config_directory() {
        // Код языка приходит из конфига, то есть из файла на диске: уход вверх
        // через `../../` читался бы чужой файл.
        let tmp = Temp::new("lang-escape");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"../../etc/passwd\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.language_error.is_some(), "обход должен быть отбит");
    }

    #[test]
    fn the_generated_config_asks_for_a_language_up_front() {
        // Ключ включён: в свежем конфиге он не закомментирован, иначе язык
        // пришлось бы сначала раскомментировать вручную.
        let tmp = Temp::new("lang-generated");
        Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let text = std::fs::read_to_string(tmp.dir().join("config.toml")).unwrap();
        assert!(
            text.contains("\nlanguage = ") || text.contains("language = \"en\""),
            "ключ языка должен быть активным: {text}"
        );
        assert!(
            !text.contains("#language"),
            "ключ языка не должен быть закомментированным: {text}"
        );
    }

    // ── Темы ───────────────────────────────────────────────────────────────────

    #[test]
    fn a_fresh_config_starts_without_a_complaint() {
        // Папки тем на первом запуске нет, и тема «default» встроенная. Если бы
        // требовался файл, редактор ругался бы на каждом первом запуске.
        let tmp = Temp::new("theme-fresh");
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(!tmp.dir().join("theme").exists(), "папки быть не должно");
        assert!(
            editor.theme_error.is_none(),
            "на первом запуске тема не должна быть претензией: {:?}",
            editor.theme_error
        );
        assert_eq!(editor.settings().theme, "default");
    }

    #[test]
    fn a_theme_from_the_config_reaches_the_settings() {
        // Ключ включён: имя из config.toml доходит до настроек, а тема с таким
        // именем в папке находится.
        let tmp = Temp::new("theme-from-config");
        write_theme(tmp.dir(), "everforest.toml");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"everforest\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.settings().theme, "everforest");
        assert!(
            editor.theme_error.is_none(),
            "тема не найдена: {:?}",
            editor.theme_error
        );
    }

    #[test]
    fn an_unknown_theme_is_reported_with_the_list() {
        // Пользователь назвал тему, которой нет: молча брать «default» —
        // значит показать не ту тему, которую он выбрал.
        let tmp = Temp::new("theme-unknown");
        write_theme(tmp.dir(), "everforest.toml");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"solarized\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let error = editor
            .theme_error
            .as_ref()
            .expect("неизвестная тема должна быть сообщена");
        let text = error.text();
        assert!(text.contains("solarized"), "{text}");
        assert!(text.contains("everforest"), "в тексте нет списка: {text}");
    }

    #[test]
    fn a_theme_name_cannot_escape_the_config_directory() {
        // Имя приходит из файла на диске: уход вверх через `../../` искал бы
        // тему вне папки тем.
        let tmp = Temp::new("theme-escape");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"../../../etc/passwd\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.theme_error.is_some(), "обход должен быть отбит");
    }

    #[test]
    fn switching_the_theme_applies_without_restart() {
        let tmp = Temp::new("theme-switch");
        write_theme(tmp.dir(), "everforest.toml");
        let mut editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.settings().theme, "default");

        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"everforest\"\n",
        )
        .unwrap();
        let reloaded = editor.reload();
        assert!(reloaded.is_ok(), "{reloaded:?}");
        assert_eq!(editor.settings().theme, "everforest");
        assert!(editor.theme_error.is_none());
    }

    #[test]
    fn a_broken_theme_does_not_stop_the_editor() {
        // Тема не нашлась, а редактор должен открыться: конфиг цел, работать
        // есть над чем. О поломке сказано, и ключ остаётся как есть.
        let tmp = Temp::new("theme-broken");
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"nope\"\n",
        )
        .unwrap();

        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.issues.is_empty(), "конфиг цел: {:?}", editor.issues);
        assert!(editor.theme_error.is_some());
        assert_eq!(editor.session().keep(), 3);
    }

    #[test]
    fn the_generated_config_asks_for_a_theme_up_front() {
        let tmp = Temp::new("theme-generated");
        Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        let text = std::fs::read_to_string(tmp.dir().join("config.toml")).unwrap();
        assert!(
            text.contains("theme = \"default\""),
            "ключ темы должен быть активным: {text}"
        );
        assert!(!text.contains("#theme"), "ключ не закомментирован: {text}");
    }

    #[test]
    fn themes_in_the_folder_do_not_apply_themselves() {
        // Папка — склад, а не выбор. Файлов может лежать сколько угодно, но без
        // имени в конфиге работает `default`: иначе тема применялась бы сама, и
        // пользователь не знал бы, почему.
        let tmp = Temp::new("theme-no-choice");
        write_theme(tmp.dir(), "everforest.toml");
        write_theme(tmp.dir(), "solarized.toml");
        // Конфиг создаётся с ключом темы по умолчанию — это `default`.
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.settings().theme, "default");
        assert!(editor.theme_error.is_none());

        // Как только имя написано — работает именно эта тема.
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\ntheme = \"everforest\"\n",
        )
        .unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.settings().theme, "everforest");
    }

    #[test]
    fn translations_in_the_folder_do_not_apply_themselves() {
        // Папка переводов — склад, как и папка тем. Файлов может лежать сколько
        // угодно, и сломанный чужой перевод не должен мешать: работает только
        // тот, чей код написан в конфиге.
        let tmp = Temp::new("lang-no-choice");
        write_translation(tmp.dir(), "ru", "\"Open file\" = \"Открыть файл\"\n");
        write_translation(tmp.dir(), "de", "\"Open file\" = \n");
        write_translation(tmp.dir(), "fr", "\"Open file\" = \"Ouvrir le fichier\"\n");

        // Язык по умолчанию — английский, и он зашит, а не взят из папки.
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "en");
        assert!(editor.language_error.is_none(), "чужие файлы не мешают");

        // Написан русский — читается русский, сломанный немецкий не мешает.
        std::fs::write(
            tmp.dir().join("config.toml"),
            "[General]\nlanguage = \"ru\"\n",
        )
        .unwrap();
        let editor = Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert_eq!(editor.language(), "ru");
        assert!(editor.language_error.is_none());
        assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
    }

    #[test]
    fn only_the_config_crate_knows_where_the_folders_are() {
        // Крейты получают готовый путь и ничего не знают о раскладке на диске.
        // Если тема или перевод снова начнут собирать путь сами, знание о папках
        // размажется по двум местам, и правка одной распадётся на другую.
        //
        // Проверяем по коду: ни в `claxis-theme`, ни в `claxis-i18n` не должно
        // быть имён папок. Имена папок — дело одного крейта.
        let forbidden = ["THEME_DIR", "LANG_DIR", "theme_dir", "lang_dir", "user_dir"];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for crate_name in ["claxis-theme", "claxis-i18n"] {
            let src = root.join(crate_name).join("src");
            for entry in std::fs::read_dir(&src).unwrap().flatten() {
                if entry.path().extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(entry.path()).unwrap();
                for name in forbidden {
                    // Только код, не комментарии и не примеры сообщений: текст
                    // про пути в сообщении — это текст, а не знание о раскладке.
                    for (n, line) in text.lines().enumerate() {
                        let line = line.trim();
                        if line.starts_with("//") || line.contains('"') {
                            continue;
                        }
                        assert!(
                            !line.contains(name),
                            "{}:{}: крейт не должен знать про папки ({name}): {line}",
                            entry.path().display(),
                            n + 1
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_config_crate_resolves_every_folder() {
        // Раз знает один крейт, он и раздаёт папки. Обе папки собираются оттуда.
        let config = crate::api::config::path::user_dir().expect("каталог находится");
        let theme = crate::api::config::path::theme_dir(&config);
        let lang = crate::api::config::path::lang_dir(&config);
        assert!(theme.ends_with("theme"), "{theme:?}");
        assert!(lang.ends_with("lang"), "{lang:?}");
        assert_ne!(theme, lang);
    }

    // ── Экран статистики ───────────────────────────────────────────────────────

    #[test]
    fn the_statistics_screen_is_configurable() {
        // Настройки экрана доходят из конфига до редактора.
        let tmp = Temp::new("savescreen");
        std::fs::write(
            tmp.dir.join("config.toml"),
            "[Savescreen]\ninclude = false\nmode = \"message\"\n",
        )
        .unwrap();
        let editor = Editor::open_in(&tmp.dir, &tmp.store).unwrap();
        assert!(!editor.settings().savescreen_include, "экран выключен");
        assert_eq!(editor.settings().savescreen_mode, "message");
    }

    #[test]
    fn the_history_usage_signature_is_an_inline_table() {
        // Подписи истории лежат во встроенной таблице, а не плоскими ключами:
        // их пишут и читают по именам `percent` и `number`.
        let tmp = Temp::new("history-usage-table");
        // Ровно так, как это пишет человек: таблица разбита по строкам.
        std::fs::write(
            tmp.dir.join("config.toml"),
            "[Savescreen.Setting]\nhistory_usage = {\n  percent = \"{percent}\",\n  number = \"{remaining}\"\n}\n",
        )
        .unwrap();
        let editor = Editor::open_in(&tmp.dir, &tmp.store).unwrap();
        let settings = editor.settings();
        assert_eq!(settings.savescreen_history_usage_percent, "{percent}");
        assert_eq!(settings.savescreen_history_usage_number, "{remaining}");
    }

    #[test]
    fn a_multiline_table_in_a_real_config_reaches_the_settings() {
        // Весь путь целиком: файл на диске, написанный руками, открывается
        // редактором, и подписи приходят в настройки без кавычек и переносов.
        let tmp = Temp::new("multiline-end-to-end");
        std::fs::write(
            tmp.dir.join("config.toml"),
            r#"[General]
language = "en"

[Savescreen]
include = true

[Savescreen.Setting]
history_usage = {
  percent = "{percent} of {max}",
  number = "{used} of {remaining}"
}
"#,
        )
        .unwrap();
        let editor = Editor::open_in(&tmp.dir, &tmp.store).unwrap();
        assert!(
            editor.issues.is_empty(),
            "конфиг с многострочной таблицей разбирается с проблемами: {:?}",
            editor.issues
        );
        let settings = editor.settings();
        assert_eq!(
            settings.savescreen_history_usage_percent,
            "{percent} of {max}"
        );
        assert_eq!(
            settings.savescreen_history_usage_number,
            "{used} of {remaining}"
        );
        assert!(settings.savescreen_include);
    }
}

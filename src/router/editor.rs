//! Всё вместе: то, с чего начинается запуск.
//!
//! Верхний уровень маршрутизатора. Держит терминал, настройки и сессию и
//! переводит настройки в параметры того, что их использует: глубина истории
//! уходит в буфер, `keep` и `persist` — в хранилище снапшотов.

use std::path::{Path, PathBuf};

use crate::api::config::schema::ConfigFile;
use crate::api::config::{Issue, generate};

use crate::api::i18n::Catalog;

use crate::router::{Applied, Session, SessionError, Settings};

/// Что не так с конфигом при запуске.
#[derive(Debug)]
pub enum StartError {
    /// Сессия не открылась.
    Session(SessionError),
    /// Каталог конфига недоступен: `{path}`, `{reason}`.
    ConfigDir {
        /// Путь к каталогу.
        path: String,
        /// Почему недоступен.
        reason: String,
    },
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
            StartError::ConfigDir { path, reason } => m.config_dir_failed(path, reason),
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
    pub watch: Option<crate::api::config::Watch>,
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
        let dir = crate::api::config::path::user_dir().ok_or_else(|| StartError::ConfigDir {
            path: "?".to_string(),
            reason: "cannot find the home directory".to_string(),
        })?;
        let store_dir = crate::api::store::StorePaths::from_env().dir;
        Self::open_in(&dir, &store_dir)
    }

    /// Конфиг задан явно, хранилище — кеш пользователя.
    pub fn open_at(dir: &Path) -> Result<Self, StartError> {
        let store_dir = crate::api::store::StorePaths::from_env().dir;
        Self::open_in(dir, &store_dir)
    }

    /// Конфиг и хранилище заданы явно — нужно в тестах.
    pub fn open_in(config_dir: &Path, store_dir: &Path) -> Result<Self, StartError> {
        // Порядок важен: сначала читается конфиг, потом под его настройки
        // создаётся сессия. Иначе настройки из файла просто никуда не денутся.
        let generated =
            generate::ensure_files(config_dir, &claxis_i18n::Catalog::new()).map_err(|e| {
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
        // редактор обязан это увидеть. Отписка не удаляет подписку, а лишь
        // говорит, что следить не вышло.
        let mut watch = crate::api::config::Watch::new(config_dir).ok();
        if let Some(w) = watch.as_mut() {
            // Подписка случилась до отметки: события между ними не считаются
            // изменением, а то первый же запуск сочтётся правкой конфига.
            w.mark_current(config_dir);
        }

        let settings = Settings::from_config(&config);
        let session = Session::open_at_with(
            store_dir,
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )
        .map_err(StartError::Session)?;

        Ok(Self {
            settings,
            session,
            issues,
            created: generated.created,
            used_last_good,
            config_dir: config_dir.to_path_buf(),
            watch,
        })
    }

    /// Собрать редактор с готовыми настройками, без чтения файлов.
    pub fn with_settings(dir: impl AsRef<Path>, settings: Settings) -> Result<Self, SessionError> {
        let session = Session::open_at_with(
            dir.as_ref(),
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )?;
        Ok(Self {
            session,
            settings,
            issues: Vec::new(),
            created: Vec::new(),
            used_last_good: false,
            config_dir: dir.as_ref().to_path_buf(),
            watch: None,
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

        let settings = Settings::from_config(&loaded.config);
        let applied = self.session.apply(&settings);
        self.settings = settings;
        self.issues = Vec::new();

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
    pub fn poll_watch(&mut self) -> Option<Reloaded> {
        let changed = self.watch.as_mut()?.changed();
        if changed.is_empty() {
            return None;
        }
        Some(self.reload())
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
        crate::api::store::SnapshotStore::open(&crate::api::store::StorePaths::new(store_dir), 1)
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
        crate::api::store::SnapshotStore::open(&crate::api::store::StorePaths::new(store_dir), 1)
            .ok()?;
    let mut docs = Vec::new();
    for &file in ConfigFile::ALL {
        let body = store.last_good(file.file_name()).ok()??;
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

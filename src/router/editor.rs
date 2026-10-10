//! Всё вместе: то, с чего начинается запуск.
//!
//! Верхний уровень маршрутизатора. Держит терминал, настройки и сессию и
//! переводит настройки в параметры того, что их использует: глубина истории
//! уходит в буфер, `keep` и `persist` — в хранилище снапшотов.

use claxis_i18n::Catalog;

use crate::router::{Session, SessionError, Settings};

/// Редактор: собранное из всех крейтов.
pub struct Editor {
    session: Session,
    settings: Settings,
}

impl Editor {
    /// Собрать редактор по настройкам.
    pub fn new(settings: Settings) -> Result<Self, SessionError> {
        let session = Session::open(
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )?;
        Ok(Self { session, settings })
    }

    /// Собрать редактор с хранилищем в заданном каталоге — нужно в тестах.
    pub fn new_at(
        dir: impl AsRef<std::path::Path>,
        settings: Settings,
    ) -> Result<Self, SessionError> {
        let session = Session::open_at_ref(
            dir.as_ref(),
            settings.snapshots_keep,
            settings.snapshots_persist,
            settings.history_depth,
        )?;
        Ok(Self { session, settings })
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
}

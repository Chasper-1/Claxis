//! Ручки крейта `claxis-store`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: типы и
//! функции. Реализация лежит в самом крейте, а связь с ручками других
//! крейтов делает маршрутизатор `crate::router`.

pub use claxis_store::defaults::{KEEP, PERSIST};
pub use claxis_store::{Error, SnapshotStore, StorePaths};

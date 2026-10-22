//! Ручки крейта `claxis-disk`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: типы и
//! функции. Реализация лежит в самом крейте, а связь с ручками других
//! крейтов делает маршрутизатор `crate::router`.

pub use claxis_disk::checksum;
pub use claxis_disk::defaults::{CACHE_DIR, KEEP, PERSIST};
pub use claxis_disk::messages;
pub use claxis_disk::watch::{BlindSpot, Changed};
pub use claxis_disk::{
    Error, Fingerprint, Snapshot, SnapshotStore, StorePaths, Watch, create_dir, file_len,
    fingerprint, read_bytes, read_text, write_blob, write_text,
};

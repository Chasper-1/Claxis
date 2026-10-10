//! Ручки крейта `claxis-config`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу.

pub use claxis_config::schema::{ConfigFile, GENERAL, find, find_in};
pub use claxis_config::{Config, Error, KEYS, KeyDef, Kind, Result};

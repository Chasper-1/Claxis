//! Ручки крейта `claxis-config`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу.

pub mod schema {
    pub use claxis_config::schema::*;
}

pub use claxis_config::generate;
pub use claxis_config::load;
pub use claxis_config::messages;
pub use claxis_config::path;
pub use claxis_config::schema::{ConfigFile, GENERAL, find, find_in};
pub use claxis_config::watch;
pub use claxis_config::{Config, Error, Issue, KEYS, KeyDef, Kind, Loaded, Problem, Result, Watch};

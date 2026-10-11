//! Ручки крейте `claxis-theme`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: список тем
//! в папке и проверку имени, на которое указывает конфиг.

pub use claxis_theme::messages;
pub use claxis_theme::{DEFAULT_THEME, EXTENSION, Error, Result, Theme, find, is_valid_name, list};

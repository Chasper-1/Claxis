//! Ручки крейтов.
//!
//! Один файл на крейт. Каждый файл отдаёт наружу только то, что крейт
//! предоставляет другим. Сами вызовы сюда не приходят: перенаправлением
//! занимается маршрутизатор, `crate::router`.

pub mod buffer;
pub mod config;
pub mod i18n;
pub mod input;
pub mod store;
pub mod term;
pub mod text;

#[cfg(test)]
mod tests;

//! Raw mode: вход без echo и построчной буферизации, выход гарантирован.

use std::io;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

/// Guard на raw mode.
///
/// При выходе и при панике терминал возвращается в исходное состояние. Без
/// этого после падения редактора терминал остаётся сломанным: ввод не
/// отображается, Enter не переводит строку.
#[derive(Debug)]
pub struct RawMode;

impl RawMode {
    /// Войти в raw mode.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Ошибка игнорируется: терминал и так закрывается, делать больше нечего.
        let _ = disable_raw_mode();
    }
}

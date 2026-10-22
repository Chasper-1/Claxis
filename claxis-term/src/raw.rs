//! Raw mode: вход без echo и построчной буферизации, выход гарантирован.

use std::io;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use crate::messages::Messages;

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

    /// Выйти из raw mode и сказать, если не вышло.
    ///
    /// Обычный выход идёт через [`RawMode::leave`]: так редактор узнаёт, если
    /// терминал остался в raw mode, и может сказать об этом пользователю. Из
    /// [`Drop`] ошибку вернуть нечем — там остаёется только прямое сообщение.
    pub fn leave(self) -> io::Result<()> {
        disable_raw_mode()
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Сюда попадаем при панике или аварийном выходе: вернуть ошибку некуда.
        // Молчать нельзя — сломанный терминал человек будет искать причину ещё
        // минуту после выхода из редактора.
        if let Err(e) = disable_raw_mode() {
            eprintln!("{}", crate::messages::En.raw_mode_failed(&e.to_string()));
        }
    }
}

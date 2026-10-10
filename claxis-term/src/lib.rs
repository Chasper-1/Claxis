//! Терминал как устройство.
//!
//! Включает kitty keyboard protocol, читает байты, разбирает в свои типы
//! событий и отдаёт наружу. Ничего не решает про смысл клавиш — это `input`.
//!
//! События отпускания приходят отдельно от нажатия, поэтому терминал обязан
//! поддерживать kitty keyboard protocol: без него «отпустил Alt» не отличить
//! от новый нажатия. Терминал без протокола получает внятное сообщение и отказ.

pub mod error;
pub mod event;
pub mod messages;
pub mod protocol;
pub mod raw;
pub mod reader;
pub mod size;

pub use error::TermError;
pub use event::{Event, Key, KeyCode, Modifiers};
pub use messages::{En, Messages};
pub use size::Size;

use std::io;

/// Терминал: протокол включён, raw mode активен, события читаются.
///
/// При выходе и при панике терминал возвращается в исходное состояние:
/// протокол выключается, raw mode снимается.
pub struct Terminal {
    raw: Option<raw::RawMode>,
}

impl Terminal {
    /// Открыть терминал: проверить протокол, включить его, войти в raw mode.
    ///
    /// Если терминал не поддерживает kitty keyboard protocol — ошибка
    /// `TermError::NoKittyProtocol`. Деградации нет: без протокола редактор
    /// работать не может.
    pub fn new() -> io::Result<Self> {
        if !protocol::supports() {
            return Err(io::Error::other(TermError::NoKittyProtocol));
        }
        protocol::enable()?;
        let raw = raw::RawMode::enter()?;
        Ok(Self { raw: Some(raw) })
    }

    /// Следующее событие от терминала.
    pub fn next_event(&mut self) -> io::Result<Event> {
        reader::next()
    }

    /// Размер терминала в символах.
    pub fn size(&self) -> io::Result<Size> {
        size::current()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Порядок важен: сначала снять raw mode, потом выключить протокол.
        // Иначе последовательности протокола попадут в канал как обычный текст.
        drop(self.raw.take());
        let _ = protocol::disable();
    }
}

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
    /// Если терминал не поддерживает kitty keyboard protocol с отчётом о типах
    /// событий — ошибка `TermError::NoKittyProtocol`. Деградации нет: без
    /// отпусканий редактор работать не может.
    ///
    /// Порядок обязателен: сначала raw mode, потом проверка протокола. В
    /// обычном режиме ввод построчно буферизован, ответ терминала без перевода
    /// строки не прочитался бы до конца ожидания — и поддержка протокола
    /// выглядела бы отсутствующей там, где она есть.
    pub fn new() -> io::Result<Self> {
        let raw = raw::RawMode::enter()?;
        if !protocol::supports() {
            return Err(io::Error::other(TermError::NoKittyProtocol));
        }
        protocol::enable()?;
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

    /// Закрыть терминал и сказать, если что-то осталось невыключенным.
    ///
    /// Обычный выход идёт через [`Terminal::close`]: редактор узнаёт, если
    /// терминал остался в raw mode или протокол не отключился, и скажет об
    /// этом пользователю. Из [`Drop`] ошибку вернуть некуда.
    pub fn close(mut self) -> io::Result<()> {
        // Порядок важен: сначала снять raw mode, потом выключить протокол.
        // Иначе последовательности протокола попадут в канал как обычный текст.
        let raw = self.raw.take();
        if let Some(raw) = raw {
            raw.leave()?;
        }
        protocol::disable()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Сюда попадаем при панике или аварийном выходе: вернуть ошибку некуда,
        // но молчать нельзя — человек будет искать причину сломанного терминала
        // уже без редактора на экране.
        if let Some(raw) = self.raw.take()
            && let Err(e) = raw.leave()
        {
            eprintln!("cannot leave raw mode: {e}");
        }
        if let Err(e) = protocol::disable() {
            eprintln!("cannot disable the keyboard protocol: {e}");
        }
    }
}

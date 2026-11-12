//! Терминал как устройство.
//!
//! Включает kitty keyboard protocol, читает байты, разбирает в свои типы
//! событий и отдаёт наружу. Ничего не решает про смысл клавиш — это `input`.
//!
//! События отпускания приходят отдельно от нажатия, поэтому терминал обязан
//! поддерживать kitty keyboard protocol: без него «отпустил Alt» не отличить
//! от новый нажатия. Терминал без протокола получает внятное сообщение и отказ.
//!
//! Этот файл только связывает модули: код лежит в файлах рядом.

pub mod error;
pub mod event;
pub mod messages;
pub mod protocol;
pub mod raw;
pub mod reader;
pub mod size;
pub mod terminal;

pub use error::TermError;
pub use event::{Event, Key, KeyCode, Modifiers, MouseButton};
pub use messages::{En, Messages};
pub use size::Size;
pub use terminal::Terminal;

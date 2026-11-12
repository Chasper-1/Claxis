//! Конфиг: набор файлов, по одному на состояние.
//!
//! Крейт ничего не знает про состояния редактора как таковые: он знает
//! список ключей из [`schema`]. Группировка по состояниям — часть схемы, а не
//! самого конфига.
//!
//! Подписи и комментарии приходят из `claxis-i18n`: конфиг зависит от
//! локализации, но не наоборот.
//!
//! Запись, чтение и наблюдение — дело крейта диска. Конфиг собирает содержимое
//! и решает, что именно записать, а как это ляжет на диск, знает не он.
//!
//! Этот файл только связывает модули: код лежит в файлах рядом.

pub mod config;
pub mod error;
pub mod generate;
pub mod load;
pub mod messages;
pub mod path;
pub mod schema;

pub use config::Config;
pub use error::{Error, Result};
pub use load::{Issue, Loaded, Problem};
pub use messages::{En, Messages};
pub use schema::TableDef;
pub use schema::{ConfigFile, GENERAL, KEYS, KeyDef, Kind};

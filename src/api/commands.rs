//! Ручки крейта `claxis-commands`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу.

pub use claxis_commands::alias::{Alias, Typed};
pub use claxis_commands::bindings::Bindings;
pub use claxis_commands::error::Error;
pub use claxis_commands::level::Level;
pub use claxis_commands::messages::{self, En, Messages};
pub use claxis_commands::name::{CommandName, SEPARATOR};
pub use claxis_commands::registry::{CommandLine, Registry};

//! Ручки крейта `claxis-core`.
//!
//! Один файл на крейт. Здесь только то, что крейт отдаёт наружу: курсор,
//! позиция, выделение, карта строк, токены и объявленные команды движения.

pub use claxis_core::{
    COMMANDS, Cursor, JumpMode, LineMap, Position, Token, TokenFilter, TokenIter, TokenKind, tokens,
};

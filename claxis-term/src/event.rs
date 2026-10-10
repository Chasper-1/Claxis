//! Свои типы событий терминала.
//!
//! Не crossterm-овские: модель ввода редактора требует различать нажатие и
//! отпускание, а у crossterm для этого нет типа. Поэтому события — свои.

use std::time::Instant;

/// Событие от терминала.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Клавиша нажата.
    KeyPressed { key: Key, at: Instant },
    /// Клавиша отпущена.
    KeyReleased { key: Key, at: Instant },
    /// Размер терминала изменился.
    Resized { cols: u16, rows: u16 },
    /// Терминал получил фокус.
    FocusGained,
    /// Терминал потерял фокус.
    FocusLost,
    /// Событие, которое редактор не разобрал.
    Unknown,
}

/// Клавиша: что нажато и с какими модификаторами.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// Что за клавиша.
    pub code: KeyCode,
    /// Модификаторы, зажатые в момент события.
    pub modifiers: Modifiers,
}

/// Модификаторы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

impl Modifiers {
    /// Без модификаторов.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        super_: false,
    };

    /// Только Ctrl.
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };

    /// Только Alt.
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    /// Только Shift.
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
}

/// Какая клавиша.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// Печатный символ.
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    /// Функциональная клавиша F1–F35.
    F(u8),
}

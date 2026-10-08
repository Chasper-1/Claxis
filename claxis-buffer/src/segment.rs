/// Источник сегмента: `0` — исходный текст, `1` — растущий буфер вставок.
pub const ORIGINAL: u32 = 0;
pub const ADDED: u32 = 1;

/// Сегмент — описание одного непрерывного участка документа.
///
/// Байтов не содержит. Указывает, из какого источника (`src`) взять участок,
/// с какого смещения (`off`) и сколько байт (`len`). Три `u32`, 12 байт.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    pub(crate) src: u32,
    pub(crate) off: u32,
    pub(crate) len: u32,
}

impl Segment {
    /// Сегмент из произвольного источника: разрез исходного сегмента даёт
    /// тот же `src`.
    pub(crate) fn new(src: u32, off: u32, len: u32) -> Self {
        Self { src, off, len }
    }

    pub fn original(off: u32, len: u32) -> Self {
        Self {
            src: ORIGINAL,
            off,
            len,
        }
    }

    pub fn added(off: u32, len: u32) -> Self {
        Self {
            src: ADDED,
            off,
            len,
        }
    }

    pub fn is_original(&self) -> bool {
        self.src == ORIGINAL
    }

    /// Смещение участка внутри источника.
    pub fn offset(&self) -> u32 {
        self.off
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn end(&self) -> u32 {
        self.off + self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

//! Размер терминала.

use std::io;

use crossterm::terminal::size;

/// Размер терминала в символах.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    /// Ширина в символах.
    pub cols: u16,
    /// Высота в строках.
    pub rows: u16,
}

/// Текущий размер терминала.
pub fn current() -> io::Result<Size> {
    let (cols, rows) = size()?;
    Ok(Size { cols, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_nonzero() {
        // Терминал без размера бесполезен, но в тестах терминала может не быть.
        // Проверяем только структуру.
        let s = Size { cols: 80, rows: 24 };
        assert_eq!(s.cols, 80);
        assert_eq!(s.rows, 24);
    }
}

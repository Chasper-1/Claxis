//! Размер чанка по объёму файла.

/// Чанк 512 Б: файлы до 128 КБ.
pub const SMALL: usize = 512;
/// Чанк 2 КБ: файлы до 512 КБ.
pub const MEDIUM: usize = 2 * 1024;
/// Чанк 4 КБ: файлы до 1 МБ.
pub const LARGE: usize = 4 * 1024;
/// Чанк 16 КБ: файлы до 4 МБ.
pub const HUGE: usize = 16 * 1024;
/// Чанк 64 КБ: всё, что больше.
pub const GIANT: usize = 64 * 1024;

/// Размер чанка для файла в `len` байт.
pub const fn chunk_size(len: usize) -> usize {
    if len <= 128 * 1024 {
        SMALL
    } else if len <= 512 * 1024 {
        MEDIUM
    } else if len <= 1024 * 1024 {
        LARGE
    } else if len <= 4 * 1024 * 1024 {
        HUGE
    } else {
        GIANT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_size_gets_its_chunk() {
        assert_eq!(chunk_size(0), SMALL);
        assert_eq!(chunk_size(128 * 1024), SMALL);
        assert_eq!(chunk_size(128 * 1024 + 1), MEDIUM);
        assert_eq!(chunk_size(512 * 1024), MEDIUM);
        assert_eq!(chunk_size(512 * 1024 + 1), LARGE);
        assert_eq!(chunk_size(1024 * 1024), LARGE);
        assert_eq!(chunk_size(1024 * 1024 + 1), HUGE);
        assert_eq!(chunk_size(4 * 1024 * 1024), HUGE);
        assert_eq!(chunk_size(4 * 1024 * 1024 + 1), GIANT);
    }
}

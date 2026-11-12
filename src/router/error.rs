//! Ошибки сессии.
//!
//! Сессия не знает языка: она отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

use std::fmt;

use crate::messages::Messages;

/// Что пошло не так в сессии.
#[derive(Debug)]
pub enum SessionError {
    /// Документ не создан: глубина истории недопустима.
    Buffer(crate::api::buffer::Error),
    /// Хранилище не открылось.
    Store { path: String, reason: String },
    /// Файл документа не прочитан.
    ReadFile { path: String, reason: String },
    /// Файл больше, чем помещается в `u32`.
    FileTooLarge { bytes: u64 },
    /// Файл не сохранён: `{path}`, `{reason}`.
    ///
    /// Здесь две разные правды: диск отказал — и тогда причина его слова, а
    /// политика записи отказалась — и тогда причина её решение. Оба текста
    /// приходят собранными: расшифровкой `save::messages` занимается слой
    /// выше, к ней сводится всё, что сохранение может сказать.
    Save { path: String, reason: String },
    /// Объявления команд не разобрались.
    ///
    /// Список команд приходит из крейтов при сборке программы, поэтому такая
    /// ошибка означает опечатку в объявлении, а не поломку у пользователя.
    Commands(crate::api::commands::Error),
}

impl SessionError {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            SessionError::Buffer(e) => messages.buffer_error(e),
            SessionError::Commands(e) => messages.commands_error(e),
            SessionError::Store { path, reason } => messages.store_failed(path, reason),
            SessionError::ReadFile { path, reason } => messages.read_file_failed(path, reason),
            SessionError::FileTooLarge { bytes } => messages.file_too_large(
                *bytes,
                *bytes as f64 / 1_000_000_000.0,
                u32::MAX as u64,
                u32::MAX as f64 / 1_000_000_000.0,
            ),
            SessionError::Save { path, reason } => messages.save_failed(path, reason),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for SessionError {}

#[cfg(test)]
mod tests {
    use super::SessionError;

    #[test]
    fn file_too_large_reports_bytes_and_limit() {
        // Файл больше u32::MAX не помещается в буфер.
        let err = SessionError::FileTooLarge {
            bytes: 5_000_000_000,
        };
        let text = err.text();

        // Точный размер в байтах.
        assert!(
            text.contains("5 000 000 000 bytes"),
            "в сообщении должен быть точный размер в байтах: {text}"
        );
        // Размер файла в десятичных гигабайтах: 5 000 000 000 / 10^9 = 5.0.
        assert!(
            text.contains("(5.0 GB)"),
            "размер файла должен быть в десятичных ГБ: {text}"
        );
        // Предел — точные байты: u32::MAX = 4 294 967 295.
        assert!(
            text.contains("\nthe maximum is 4 294 967 295 bytes"),
            "предел должен быть в точных байтах: {text}"
        );
        // Гигабайты приложены для удобства: 4.29 ГБ, а не 4.0.
        assert!(
            text.contains("(4.3 GB)"),
            "предел в ГБ должен быть 4.3, а не 4.0: {text}"
        );
    }

    #[test]
    fn gigabytes_are_decimal_not_binary() {
        // Гигабайт считается в 10^9. Если считать в 2^30, то получится 4.0 и
        // пользователю покажут неверный предел.
        let limit_gb = u32::MAX as f64 / 1_000_000_000.0;
        assert!(
            (limit_gb - 4.294967295).abs() < 1e-9,
            "предел должен быть 4.294967295 ГБ, получено {limit_gb}"
        );
        assert!(
            (u32::MAX as f64 / 1_073_741_824.0 - 4.0).abs() < 1e-9,
            "деление на 2^30 даёт 4.0 — это гибибайты, а не гигабайты"
        );
    }

    #[test]
    fn big_numbers_are_split_into_readable_parts() {
        use crate::api::text::group_digits;

        // Без разделителей длинное число не читается.
        assert_eq!(group_digits(4_294_967_295), "4 294 967 295");
        assert_eq!(group_digits(1_000), "1 000");
        assert_eq!(group_digits(1_234_567_890), "1 234 567 890");
        // Числа короче трёх разрядов не трогаем.
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(100), "100");
        assert_eq!(group_digits(0), "0");
    }
}

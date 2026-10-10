//! Сообщения об ошибках сессии.
//!
//! Сессия не знает языка: она отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

/// Каталог сообщений об ошибках сессии.
///
/// Места под значения: `{path}`, `{reason}`, `{bytes}`, `{limit_gb}`.
pub trait Messages {
    /// Хранилище не открылось.
    fn store_failed(&self, path: &str, reason: &str) -> String;
    /// Файл документа не прочитан.
    fn read_file_failed(&self, path: &str, reason: &str) -> String;
    /// Ошибка буфера: делегирует в каталог буфера.
    fn buffer_error(&self, error: &claxis_buffer::Error) -> String;
    /// Файл больше, чем помещается в `u32`.
    ///
    /// Байты — точное число, гигабайты приложены для удобства. Считаются в
    /// десятичных, 10⁹, как обычно.
    ///
    /// `{bytes}` и `{limit_bytes}` — точные размеры, `{size_gb}` и
    /// `{limit_gb}` — те же числа в гигабайтах.
    fn file_too_large(&self, bytes: u64, size_gb: f64, limit_bytes: u64, limit_gb: f64) -> String;
}

/// Разделитель разрядов в числах.
///
/// Пробел: он читается одинаково в любом языке и не путается с точкой в
/// десятичной дроби, в отличие от точки или запятой.
const THOUSANDS: char = ' ';

/// Разбить число на разряды: `4294967295` становится `4 294 967 295`.
///
/// Без разбивки длинное число не читается — не видно, где миллиард. Правило
/// одно на все сообщения редактора, а не на каждый крейт своё.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    if digits.len() <= 3 {
        return digits;
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        // Разделитель ставим там, где справа остаётся кратное трём число цифр.
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(THOUSANDS);
        }
        out.push(ch);
    }
    out
}

/// Подставить значения вместо имён в фигурных скобках.
pub fn substitute(text: &str, values: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn store_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot open store at {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn read_file_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot read file {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn buffer_error(&self, error: &claxis_buffer::Error) -> String {
        error.message(&claxis_buffer::En)
    }

    fn file_too_large(&self, bytes: u64, size_gb: f64, limit_bytes: u64, limit_gb: f64) -> String {
        substitute(
            "file is too large: {bytes} bytes ({size_gb} GB),\n\
             the maximum is {limit_bytes} bytes ({limit_gb} GB)",
            &[
                ("bytes", group_digits(bytes)),
                ("size_gb", format!("{size_gb:.1}")),
                ("limit_bytes", group_digits(limit_bytes)),
                ("limit_gb", format!("{limit_gb:.1}")),
            ],
        )
    }
}

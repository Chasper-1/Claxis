//! Сообщения об ошибках терминала.
//!
//! Терминал не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский.

/// Каталог сообщений об ошибках терминала.
pub trait Messages {
    /// Терминал не поддерживает kitty keyboard protocol.
    fn no_kitty_protocol(&self) -> String;
}

/// Подставить значения вместо имён в фигурных скобках.
pub fn substitute(text: &str, values: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// Разделитель разрядов в числах.
///
/// Пробел: он читается одинаково в любом языке и не путается с точкой в
/// десятичной дроби, в отличие от точки или запятой.
const THOUSANDS: char = ' ';

/// Разбить число на разряды: `4294967295` становится `4 294 967 295`.
///
/// Правило одно на все сообщения редактора, а не на каждый крейт своё.
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

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn no_kitty_protocol(&self) -> String {
        "terminal does not support kitty keyboard protocol. \
         A terminal with support is required: kitty, WezTerm, foot, Ghostty, \
         Windows Terminal configuration. The editor cannot work without the protocol."
            .to_string()
    }
}

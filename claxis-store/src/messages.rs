//! Сообщения об ошибках хранилища.
//!
//! Хранилище не знает языка: оно отдаёт ошибку со структурными данными, а
//! текст собирает каталог. По умолчанию — английский.

/// Каталог сообщений об ошибках хранилища.
///
/// Места под значения: `{path}`, `{reason}`, `{file}`.
pub trait Messages {
    /// Файл базы не открылся.
    fn open_failed(&self, path: &str, reason: &str) -> String;
    /// Каталог для базы не создался.
    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String;
    /// Запрос к базе не выполнился.
    fn query_failed(&self, reason: &str) -> String;
    /// Снапшота для файла нет.
    fn no_snapshot(&self, file: &str) -> String;
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
    fn open_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot open store at {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn prepare_dir_failed(&self, path: &str, reason: &str) -> String {
        substitute(
            "cannot create store directory {path}: {reason}",
            &[("path", path.to_string()), ("reason", reason.to_string())],
        )
    }

    fn query_failed(&self, reason: &str) -> String {
        substitute(
            "store query failed: {reason}",
            &[("reason", reason.to_string())],
        )
    }

    fn no_snapshot(&self, file: &str) -> String {
        substitute(
            "no snapshot stored for {file}",
            &[("file", file.to_string())],
        )
    }
}

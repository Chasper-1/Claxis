//! Сообщения об ошибках буфера.
//!
//! Буфер не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский, другие языки подключаются
//! снаружи: редактор берёт каталог из конфига и зовёт [`crate::buffer::Error::message`].
//!
//! Имена в фигурных скобках — это места под значения. Написано в конфиге
//! `пул исчерпан: узлов {capacity}, глубина {depth}` — на экране будет `пул
//! исчерпан: узлов 408, глубина 100`. Переводчик ставит имена туда, куда надо
//! по правилам своего языка, буфер подставляет вместо них числа.

/// Каталог сообщений об ошибках буфера.
///
/// Места под значения: `{anchor}`, `{end}`, `{doc_len}`, `{depth}`, `{min}`,
/// `{max}`, `{capacity}`.
pub trait Messages {
    /// Вставка или удаление вышли за границы документа.
    fn out_of_bounds(&self, anchor: u32, end: u32, doc_len: u32) -> String;

    /// Глубина истории вне допустимого диапазона.
    fn invalid_history_depth(&self, depth: u32, min: u32, max: u32) -> String;

    /// Пул узлов дерева исчерпан: документ вырос сверх рассчитанного предела.
    fn tree_pool_exhausted(&self, capacity: usize, depth: u32) -> String;

    /// В переводе встретилось имя, которого нет среди известных.
    ///
    /// `{got}` — что пришло, `{expected}` — какие имена допустимы.
    fn bad_placeholder(&self, got: &str, expected: &[&str]) -> String;

    /// В переводе `{` без пары `}`. `{at}` — хвост начиная с незакрытой скобки.
    fn unclosed_brace(&self, at: &str) -> String;
}

/// Ошибка разбора перевода: неизвестное имя или незакрытая скобка.
///
/// Тип данных, а не готовая строка: конфиг сам соберёт из него сообщение на
/// своём языке и подставит куда надо.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// Имя в фигурных скобках не совпало ни с одним известным.
    UnknownName {
        /// Что пришло в переводе, без скобок.
        got: &'static str,
        /// Какие имена здесь допустимы.
        expected: &'static [&'static str],
    },
    /// Скобка `{` без пары `}`.
    UnclosedBrace {
        /// Хвост начиная с незакрытой скобки.
        at: &'static str,
    },
}

impl ParseError {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            ParseError::UnknownName { got, expected } => messages.bad_placeholder(got, expected),
            ParseError::UnclosedBrace { at } => messages.unclosed_brace(at),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&En)
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for ParseError {}

/// Подставить значения вместо имён в фигурных скобках.
///
/// Публичная: каталог из конфига пользуется ею же, чтобы подставить значения
/// в текст, написанный переводчиком.
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

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn out_of_bounds(&self, anchor: u32, end: u32, doc_len: u32) -> String {
        substitute(
            "range {anchor}..{end} is outside the document of length {doc_len}",
            &[
                ("anchor", group_digits(anchor as u64)),
                ("end", group_digits(end as u64)),
                ("doc_len", group_digits(doc_len as u64)),
            ],
        )
    }

    fn invalid_history_depth(&self, depth: u32, min: u32, max: u32) -> String {
        substitute(
            "invalid history depth {depth}: must be between {min} and {max} records",
            &[
                ("depth", group_digits(depth as u64)),
                ("min", group_digits(min as u64)),
                ("max", group_digits(max as u64)),
            ],
        )
    }

    fn tree_pool_exhausted(&self, capacity: usize, depth: u32) -> String {
        substitute(
            "buffer tree pool exhausted: {capacity} nodes is not enough for depth {depth}",
            &[
                ("capacity", group_digits(capacity as u64)),
                ("depth", group_digits(depth as u64)),
            ],
        )
    }

    fn bad_placeholder(&self, got: &str, expected: &[&str]) -> String {
        substitute(
            "unknown placeholder {{{got}}}: expected one of {expected}",
            &[("got", got.to_string()), ("expected", expected.join(", "))],
        )
    }

    fn unclosed_brace(&self, at: &str) -> String {
        substitute("unclosed brace at \"{at}\"", &[("at", at.to_string())])
    }
}

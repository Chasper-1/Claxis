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

/// Английские сообщения — язык по умолчанию.
#[derive(Clone, Copy, Debug, Default)]
pub struct En;

impl Messages for En {
    fn out_of_bounds(&self, anchor: u32, end: u32, doc_len: u32) -> String {
        substitute(
            "range {anchor}..{end} is outside the document of length {doc_len}",
            &[
                ("anchor", anchor.to_string()),
                ("end", end.to_string()),
                ("doc_len", doc_len.to_string()),
            ],
        )
    }

    fn invalid_history_depth(&self, depth: u32, min: u32, max: u32) -> String {
        substitute(
            "invalid history depth {depth}: must be between {min} and {max} records",
            &[
                ("depth", depth.to_string()),
                ("min", min.to_string()),
                ("max", max.to_string()),
            ],
        )
    }

    fn tree_pool_exhausted(&self, capacity: usize, depth: u32) -> String {
        substitute(
            "buffer tree pool exhausted: {capacity} nodes is not enough for depth {depth}",
            &[
                ("capacity", capacity.to_string()),
                ("depth", depth.to_string()),
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

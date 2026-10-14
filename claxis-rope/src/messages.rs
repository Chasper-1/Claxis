//! Сообщения об ошибках rope.
//!
//! Крейт не знает языка: он отдаёт ошибку со структурными данными, а текст
//! собирает каталог. По умолчанию — английский, другие языки подключаются
//! снаружи: редактор берёт каталог из конфига и зовёт
//! [`crate::Error::message`].
//!
//! Имена в фигурных скобках — это места под значения. Написано в переводе
//! `вне документа: {anchor}..{end}, длина {doc_len}` — на экране будет
//! `вне документа: 1000..2000, длина 1500`. Переводчик ставит имена туда,
//! куда надо по правилам своего языка, rope подставляет вместо них числа.

/// Каталог сообщений об ошибках rope.
///
/// Места под значения: `{anchor}`, `{end}`, `{doc_len}`, `{depth}`, `{min}`,
/// `{version}`, `{kind}`, `{got}`, `{expected}`, `{at}`.
pub trait Messages {
    /// Вставка или удаление вышли за границы документа.
    fn out_of_bounds(&self, anchor: u32, end: u32, doc_len: u32) -> String;

    /// Глубина истории вне допустимого диапазона.
    ///
    /// Потолка нет: история растёт вместе с правками. Отвергается только ноль
    /// — с ним снимался бы снапшот после каждой правки.
    fn invalid_history_depth(&self, depth: u32, min: u32) -> String;

    /// В истории лежит не формат rope: чужой блоб.
    fn not_our_history(&self) -> String;

    /// Версия истории не та, что ждёт крейт.
    ///
    /// `{version}` — что пришло.
    fn unknown_history_version(&self, version: u8) -> String;

    /// История оборвана: данных меньше, чем записано в заголовке.
    fn truncated_history(&self) -> String;

    /// Запись истории неизвестного вида.
    ///
    /// `{kind}` — что пришло в байте вида.
    fn unknown_record_kind(&self, kind: u8) -> String;

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

pub use claxis_text::{group_digits, substitute};

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

    fn invalid_history_depth(&self, depth: u32, min: u32) -> String {
        substitute(
            "invalid history depth {depth}: must be at least {min} records",
            &[
                ("depth", group_digits(depth as u64)),
                ("min", group_digits(min as u64)),
            ],
        )
    }

    fn not_our_history(&self) -> String {
        substitute("not a rope history blob", &[])
    }

    fn unknown_history_version(&self, version: u8) -> String {
        substitute(
            "unknown rope history version {version}: expected version 1",
            &[("version", version.to_string())],
        )
    }

    fn truncated_history(&self) -> String {
        substitute("rope history is truncated", &[])
    }

    fn unknown_record_kind(&self, kind: u8) -> String {
        substitute(
            "unknown record kind {kind} in rope history",
            &[("kind", kind.to_string())],
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

/// Все сообщения крейте с примерами значений.
///
/// Нужны для отладочного вывода и для переводчика: он видит не ключ, а то,
/// как сообщение выглядит на самом деле, с настоящими числами вместо
/// имён в фигурных скобках.
pub fn samples() -> Vec<(&'static str, String)> {
    let m = En;
    vec![
        ("out_of_bounds", m.out_of_bounds(1_000, 2_000, 1_500)),
        ("invalid_history_depth", m.invalid_history_depth(0, 1)),
        ("not_our_history", m.not_our_history()),
        ("unknown_history_version", m.unknown_history_version(7)),
        ("truncated_history", m.truncated_history()),
        ("unknown_record_kind", m.unknown_record_kind(9)),
        (
            "bad_placeholder",
            m.bad_placeholder("capasity", &["capacity", "depth"]),
        ),
        ("unclosed_brace", m.unclosed_brace("{capacity")),
    ]
}

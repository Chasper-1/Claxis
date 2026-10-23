//! Токены: буквы, числа и символы.
//!
//! Токен — это отрезок одного вида. Буквы, цифры и символы разделены
//! намеренно: курсор должен уметь прыгать по каждому виду отдельно, а
//! сочетания собираются фильтром, а не режимом.
//!
//! Никакого списка токенов не хранится. Обход идёт по байтам через границы
//! графем, поэтому эмодзи и буква с комбинирующим знаком остаются целыми.

#[cfg(test)]
use std::iter::Peekable;

use claxis_text::grapheme;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Вид токена.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Буквы и комбинирующиеся знаки при них.
    Word,
    /// Цифры.
    Number,
    /// Всё остальное, кроме пробельных: знаки препинания, эмодзи, мусор.
    Symbol,
}

/// Токен: байтовый отрезок и его вид.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// Начало отрезка.
    pub start: usize,
    /// Конец отрезка, не включая его.
    pub end: usize,
    /// Вид токена.
    pub kind: TokenKind,
}

/// Какие виды токенов участвуют в прыжке.
///
/// Одна команда включает один вид, макрос `/` объединяет несколько. Пустой
/// фильтр не двигает курсор.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenFilter {
    word: bool,
    number: bool,
    symbol: bool,
}

impl TokenFilter {
    /// Только буквы.
    pub const fn word() -> Self {
        Self {
            word: true,
            number: false,
            symbol: false,
        }
    }

    /// Только цифры.
    pub const fn number() -> Self {
        Self {
            word: false,
            number: true,
            symbol: false,
        }
    }

    /// Только символы.
    pub const fn symbol() -> Self {
        Self {
            word: false,
            number: false,
            symbol: true,
        }
    }

    /// Все три вида.
    pub const fn all() -> Self {
        Self {
            word: true,
            number: true,
            symbol: true,
        }
    }

    /// Входит ли вид в фильтр.
    pub const fn contains(self, kind: TokenKind) -> bool {
        match kind {
            TokenKind::Word => self.word,
            TokenKind::Number => self.number,
            TokenKind::Symbol => self.symbol,
        }
    }

    /// Объединить два фильтра: то, что нужно макросу `/`.
    pub const fn union(self, other: Self) -> Self {
        Self {
            word: self.word || other.word,
            number: self.number || other.number,
            symbol: self.symbol || other.symbol,
        }
    }

    /// Пустой фильтр.
    pub const fn is_empty(self) -> bool {
        !self.word && !self.number && !self.symbol
    }
}

/// Куда прыгать, если курсор стоит внутри токена.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JumpMode {
    /// Допрыгнуть до конца текущего токена. По умолчанию.
    #[default]
    FinishCurrent,
    /// Пропустить текущий токен и встать в начало следующего.
    NextStart,
}

/// Обход токенов куска текста.
///
/// Пробельные графемы — разделители, а не токены. Они пропускаются, но
/// границы соседних токенов от этого не склеиваются.
pub struct TokenIter<'a> {
    line: Line<'a>,
    at: usize,
}

impl Iterator for TokenIter<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        loop {
            if self.at >= self.line.len() {
                return None;
            }
            let start = self.at;
            let end = self.line.next(start);
            self.at = end;
            if is_space(self.line.bytes(), start, end) {
                continue;
            }
            let kind = kind_of(self.line.bytes(), start, end);
            while self.at < self.line.len() {
                let next_end = self.line.next(self.at);
                if is_space(self.line.bytes(), self.at, next_end) {
                    break;
                }
                if kind_of(self.line.bytes(), self.at, next_end) != kind {
                    break;
                }
                self.at = next_end;
            }
            return Some(Token {
                start,
                end: self.at,
                kind,
            });
        }
    }
}

/// Обойти токены.
pub fn tokens(text: &[u8]) -> TokenIter<'_> {
    TokenIter {
        line: Line::new(text),
        at: 0,
    }
}

/// Кусок текста с локальной проверкой UTF-8.
///
/// Проверять всю строку на каждом шаге — значит платить за длину строки на
/// каждый прыжок. Здесь проверяется только окно вокруг графемы, а ASCII
/// вообще не требует проверки. Окно растёт, если графема в него не влезла.
#[derive(Clone, Copy)]
struct Line<'a> {
    bytes: &'a [u8],
}

impl<'a> Line<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    fn next(&self, at: usize) -> usize {
        let len = self.len();
        let at = at.min(len);
        if at >= len {
            return len;
        }
        if self.bytes[at] == b'\r' && self.bytes.get(at + 1) == Some(&b'\n') {
            return at + 2;
        }
        if self.bytes[at].is_ascii() && self.bytes.get(at + 1).is_none_or(|next| next.is_ascii()) {
            return at + 1;
        }
        let mut span = 16;
        loop {
            let end = (at + span).min(len);
            match std::str::from_utf8(&self.bytes[at..end]) {
                Ok(text) => {
                    if let Some(grapheme) = text.graphemes(true).next() {
                        let next = at + grapheme.len();
                        if next < end || end == len {
                            return next;
                        }
                    } else {
                        return len;
                    }
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid == 0 {
                        return at + 1;
                    }
                    if error.error_len().is_none() && end < len {
                        // Окно разрезало символ, нужен кусок побольше.
                    } else if let Ok(text) = std::str::from_utf8(&self.bytes[at..at + valid])
                        && let Some(grapheme) = text.graphemes(true).next()
                    {
                        return at + grapheme.len();
                    } else {
                        return at + 1;
                    }
                }
            }
            if end == len {
                return len;
            }
            span = span.saturating_mul(2);
        }
    }

    fn prev(&self, at: usize) -> usize {
        let len = self.len();
        let at = at.min(len);
        if at == 0 {
            return 0;
        }
        let after_crlf = at >= 2 && self.bytes[at - 2] == b'\r' && self.bytes[at - 1] == b'\n';
        if self.bytes[at - 1].is_ascii()
            && !after_crlf
            && !(self.bytes[at - 1] == b'\r' && self.bytes.get(at) == Some(&b'\n'))
            && self.bytes.get(at).is_none_or(|next| next.is_ascii())
        {
            return at - 1;
        }
        let mut end = at;
        while end < len && self.bytes[end] & 0b1100_0000 == 0b1000_0000 {
            end += 1;
        }
        let mut span = 16;
        loop {
            let mut begin = end.saturating_sub(span);
            while begin > 0 && self.bytes[begin] & 0b1100_0000 == 0b1000_0000 {
                begin -= 1;
            }
            let Ok(text) = std::str::from_utf8(&self.bytes[begin..end]) else {
                return grapheme::prev(self.bytes, at);
            };
            let mut cursor = GraphemeCursor::new(end - begin, text.len(), true);
            match cursor.prev_boundary(text, 0) {
                Ok(Some(previous)) => {
                    if previous > 0 || begin == 0 {
                        return begin + previous;
                    }
                    span = span.saturating_mul(2);
                }
                Ok(None) => {
                    if begin == 0 {
                        return 0;
                    }
                    span = span.saturating_mul(2);
                }
                Err(_) => {
                    if begin == 0 {
                        return grapheme::prev(self.bytes, at);
                    }
                    span = span.saturating_mul(2);
                }
            }
        }
    }

    fn is_boundary(&self, at: usize) -> bool {
        at == 0 || at >= self.len() || self.next(self.prev(at)) == at
    }

    fn grapheme_start(&self, at: usize) -> usize {
        let at = at.min(self.len());
        if at == 0 || self.is_boundary(at) {
            at
        } else {
            self.prev(at)
        }
    }
}

/// Начало следующего подходящего токена или его конец.
///
/// `FinishCurrent` допрыгивает до конца текущего токена, `NextStart`
/// перепрыгивает его и встаёт в начало следующего. Если подходящих токенов
/// впереди нет, возвращается конец куска.
pub fn next(text: &[u8], at: usize, filter: TokenFilter, mode: JumpMode) -> usize {
    let at = at.min(text.len());
    if filter.is_empty() {
        return at;
    }
    let line = Line::new(text);
    let Some(token) = token_at_or_after(line, at) else {
        return line.len();
    };
    if filter.contains(token.kind) {
        let end = run_end(line, token, filter);
        if at >= token.start && at < token.end {
            return match mode {
                JumpMode::FinishCurrent => end,
                JumpMode::NextStart => {
                    next_filtered(line, end, filter).map_or(line.len(), |next| next.start)
                }
            };
        }
        return match mode {
            JumpMode::FinishCurrent => end,
            JumpMode::NextStart => token.start,
        };
    }
    match next_filtered(line, token.end, filter) {
        Some(next) => match mode {
            JumpMode::FinishCurrent => run_end(line, next, filter),
            JumpMode::NextStart => next.start,
        },
        None => line.len(),
    }
}

/// Начало предыдущего подходящего токена.
///
/// `FinishCurrent` возвращает начало текущего токена, `NextStart` — начало
/// предыдущего. Если позади ничего нет, возвращается начало куска.
pub fn prev(text: &[u8], at: usize, filter: TokenFilter, mode: JumpMode) -> usize {
    let at = at.min(text.len());
    if filter.is_empty() {
        return at;
    }
    let line = Line::new(text);
    let inside =
        token_at_or_before(line, at).filter(|token| filter.contains(token.kind) && token.end >= at);
    if let Some(token) = inside {
        let start = run_start(line, token, filter);
        return match mode {
            JumpMode::FinishCurrent => start,
            JumpMode::NextStart => previous_filtered(line, start, filter)
                .map_or(0, |previous| run_start(line, previous, filter)),
        };
    }
    previous_filtered(line, at, filter).map_or(0, |token| run_start(line, token, filter))
}

/// Токен, в котором стоит `at`, либо первый токен после него.
fn token_at_or_after(line: Line<'_>, at: usize) -> Option<Token> {
    let mut pos = at;
    while pos < line.len() {
        let (start, end) = grapheme_at(line, pos)?;
        if !is_space(line.bytes(), start, end) {
            return Some(expand(line, start, end));
        }
        pos = end;
    }
    None
}

/// Токен, который заканчивается на `at` или содержит его, либо предыдущий.
fn token_at_or_before(line: Line<'_>, at: usize) -> Option<Token> {
    let mut pos = at.min(line.len());
    while pos > 0 {
        let start = if line.is_boundary(pos) {
            line.prev(pos)
        } else {
            line.grapheme_start(pos)
        };
        let end = line.next(start);
        if is_space(line.bytes(), start, end) {
            pos = start;
            continue;
        }
        return Some(expand(line, start, end));
    }
    None
}

/// Границы графемы, в которую попадает `at`, или которая начинается в `at`.
fn grapheme_at(line: Line<'_>, at: usize) -> Option<(usize, usize)> {
    if at >= line.len() {
        return None;
    }
    let start = line.grapheme_start(at);
    Some((start, line.next(start)))
}

/// Развернуть графему до целого токена того же вида.
fn expand(line: Line<'_>, start: usize, end: usize) -> Token {
    let bytes = line.bytes();
    let kind = kind_of(bytes, start, end);
    let mut token_start = start;
    loop {
        let previous = line.prev(token_start);
        if previous == token_start {
            break;
        }
        if is_space(bytes, previous, token_start) || kind_of(bytes, previous, token_start) != kind {
            break;
        }
        token_start = previous;
    }
    let mut token_end = end;
    loop {
        let next = line.next(token_end);
        if next == token_end {
            break;
        }
        if is_space(bytes, token_end, next) || kind_of(bytes, token_end, next) != kind {
            break;
        }
        token_end = next;
    }
    Token {
        start: token_start,
        end: token_end,
        kind,
    }
}

/// Конец непрерывного отрезка подходящих токенов.
fn run_end(line: Line<'_>, token: Token, filter: TokenFilter) -> usize {
    let mut end = token.end;
    while let Some(next) = token_at_or_after(line, end) {
        if next.start != end || !filter.contains(next.kind) {
            break;
        }
        end = next.end;
    }
    end
}

/// Начало непрерывного отрезка подходящих токенов.
fn run_start(line: Line<'_>, token: Token, filter: TokenFilter) -> usize {
    let mut start = token.start;
    while let Some(previous) = token_at_or_before(line, start) {
        if previous.end != start || !filter.contains(previous.kind) {
            break;
        }
        start = previous.start;
    }
    start
}

/// Первый подходящий токен начиная с `at`.
fn next_filtered(line: Line<'_>, at: usize, filter: TokenFilter) -> Option<Token> {
    let mut pos = at;
    while let Some(token) = token_at_or_after(line, pos) {
        if filter.contains(token.kind) {
            return Some(token);
        }
        pos = token.end;
    }
    None
}

/// Последний подходящий токен до `at`.
fn previous_filtered(line: Line<'_>, at: usize, filter: TokenFilter) -> Option<Token> {
    let mut pos = at;
    while let Some(token) = token_at_or_before(line, pos) {
        if filter.contains(token.kind) {
            return Some(token);
        }
        pos = token.start;
    }
    None
}

/// Обход непрерывных отрезков из подходящих токенов.
///
/// Два соседних токена склеиваются, только если между ними нет пробела и оба
/// входят в фильтр. Так `word / symbol` перепрыгивает `foo_bar` целиком, а
/// `word | symbol` делает два отдельных прыжка.
#[cfg(test)]
struct RunIter<'a> {
    tokens: Peekable<TokenIter<'a>>,
    filter: TokenFilter,
}

#[cfg(test)]
fn runs(text: &[u8], filter: TokenFilter) -> RunIter<'_> {
    RunIter {
        tokens: tokens(text).peekable(),
        filter,
    }
}

#[cfg(test)]
impl Iterator for RunIter<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        loop {
            let first = self.tokens.next()?;
            if !self.filter.contains(first.kind) {
                continue;
            }
            let start = first.start;
            let mut end = first.end;
            while let Some(next) = self.tokens.peek() {
                if !self.filter.contains(next.kind) || next.start != end {
                    break;
                }
                end = next.end;
                self.tokens.next();
            }
            return Some((start, end));
        }
    }
}

/// Первый символ графемы.
fn first_char(text: &[u8], start: usize, end: usize) -> Option<char> {
    std::str::from_utf8(&text[start..end])
        .ok()
        .and_then(|text| text.chars().next())
}

/// Пробельная ли графема.
fn is_space(text: &[u8], start: usize, end: usize) -> bool {
    first_char(text, start, end).is_some_and(char::is_whitespace)
}

/// Вид графемы по её первому символу.
fn kind_of(text: &[u8], start: usize, end: usize) -> TokenKind {
    match first_char(text, start, end) {
        Some(c) if c.is_alphabetic() => TokenKind::Word,
        Some(c) if c.is_numeric() => TokenKind::Number,
        _ => TokenKind::Symbol,
    }
}

#[cfg(test)]
mod tests {
    use claxis_text::grapheme;

    use super::{JumpMode, TokenFilter, TokenKind, next, prev, runs, tokens};

    fn seen(text: &str) -> Vec<(TokenKind, &str)> {
        tokens(text.as_bytes())
            .map(|token| (token.kind, &text[token.start..token.end]))
            .collect()
    }

    #[test]
    fn letters_digits_and_symbols_are_different_tokens() {
        assert_eq!(
            seen("foo_bar123"),
            [
                (TokenKind::Word, "foo"),
                (TokenKind::Symbol, "_"),
                (TokenKind::Word, "bar"),
                (TokenKind::Number, "123"),
            ]
        );
    }

    #[test]
    fn a_run_of_symbols_is_one_token() {
        assert_eq!(
            seen("-_.->()"),
            [(TokenKind::Symbol, "-_.->()")],
            "символы идут одним отрезком"
        );
    }

    #[test]
    fn a_dot_splits_a_number() {
        assert_eq!(
            seen("3.14"),
            [
                (TokenKind::Number, "3"),
                (TokenKind::Symbol, "."),
                (TokenKind::Number, "14"),
            ]
        );
    }

    #[test]
    fn whitespace_is_not_a_token() {
        assert_eq!(
            seen("  foo\tbar\n"),
            [(TokenKind::Word, "foo"), (TokenKind::Word, "bar")]
        );
    }

    #[test]
    fn cyrillic_letters_are_words() {
        assert_eq!(
            seen("привет_мир"),
            [
                (TokenKind::Word, "привет"),
                (TokenKind::Symbol, "_"),
                (TokenKind::Word, "мир"),
            ]
        );
    }

    #[test]
    fn an_emoji_is_one_symbol_token() {
        assert_eq!(seen("👨‍👩‍👧‍👦!"), [(TokenKind::Symbol, "👨‍👩‍👧‍👦!")]);
    }

    #[test]
    fn a_combining_mark_does_not_split_a_word() {
        assert_eq!(seen("e\u{301}x"), [(TokenKind::Word, "e\u{301}x")]);
    }

    #[test]
    fn a_broken_byte_is_a_symbol() {
        let text = b"a\xFFb";
        let quiet: Vec<_> = tokens(text)
            .map(|token| (token.kind, token.start, token.end))
            .collect();
        assert_eq!(
            quiet,
            [
                (TokenKind::Word, 0, 1),
                (TokenKind::Symbol, 1, 2),
                (TokenKind::Word, 2, 3),
            ]
        );
    }

    #[test]
    fn word_right_finishes_the_current_word() {
        let text = b"foo_bar";
        assert_eq!(
            next(text, 0, TokenFilter::word(), JumpMode::FinishCurrent),
            3
        );
        assert_eq!(
            next(text, 3, TokenFilter::symbol(), JumpMode::FinishCurrent),
            4
        );
    }

    #[test]
    fn word_then_symbol_does_not_reach_the_next_word() {
        // Ровно случай из задачи: после двух команд курсор на `foo_`, `bar`
        // ещё не пройден.
        let text = b"foo_bar";
        let after_word = next(text, 0, TokenFilter::word(), JumpMode::FinishCurrent);
        let after_symbol = next(
            text,
            after_word,
            TokenFilter::symbol(),
            JumpMode::FinishCurrent,
        );
        assert_eq!(&text[..after_symbol], b"foo_");
        assert_eq!(after_symbol, 4);
    }

    #[test]
    fn a_combined_filter_swallows_word_and_symbol_together() {
        let text = b"foo_bar";
        let filter = TokenFilter::word().union(TokenFilter::symbol());
        assert_eq!(next(text, 0, filter, JumpMode::FinishCurrent), 7);
    }

    #[test]
    fn number_jumps_stop_at_the_number() {
        let text = b"foo123bar";
        assert_eq!(
            next(text, 0, TokenFilter::number(), JumpMode::FinishCurrent),
            6
        );
        assert_eq!(
            next(text, 6, TokenFilter::word(), JumpMode::FinishCurrent),
            9
        );
    }

    #[test]
    fn next_start_skips_the_current_token() {
        let text = b"foo bar";
        assert_eq!(
            next(text, 1, TokenFilter::word(), JumpMode::NextStart),
            4,
            "из середины слова — в начало следующего"
        );
        assert_eq!(next(text, 0, TokenFilter::word(), JumpMode::NextStart), 4);
    }

    #[test]
    fn finish_current_goes_to_the_end_of_the_word() {
        let text = b"foo bar";
        assert_eq!(
            next(text, 1, TokenFilter::word(), JumpMode::FinishCurrent),
            3
        );
        assert_eq!(
            next(text, 3, TokenFilter::word(), JumpMode::FinishCurrent),
            7,
            "с пробела — конец следующего слова"
        );
    }

    #[test]
    fn left_finishes_the_current_token() {
        let text = b"foo_bar";
        assert_eq!(
            prev(text, 6, TokenFilter::word(), JumpMode::FinishCurrent),
            4
        );
        assert_eq!(
            prev(text, 4, TokenFilter::word(), JumpMode::FinishCurrent),
            0,
            "с начала слова — начало предыдущего"
        );
    }

    #[test]
    fn left_can_skip_the_current_token() {
        let text = b"foo_bar";
        assert_eq!(prev(text, 6, TokenFilter::word(), JumpMode::NextStart), 0);
    }

    #[test]
    fn there_is_nowhere_to_go_beyond_the_edges() {
        let text = b"foo";
        assert_eq!(
            next(text, 99, TokenFilter::all(), JumpMode::FinishCurrent),
            3
        );
        assert_eq!(
            prev(text, 0, TokenFilter::all(), JumpMode::FinishCurrent),
            0
        );
    }

    #[test]
    fn an_empty_filter_does_not_move() {
        let text = b"foo bar";
        let empty = TokenFilter::default();
        assert_eq!(next(text, 2, empty, JumpMode::FinishCurrent), 2);
        assert_eq!(prev(text, 2, empty, JumpMode::FinishCurrent), 2);
    }

    #[test]
    fn filters_combine() {
        let filter = TokenFilter::word().union(TokenFilter::number());
        assert!(filter.contains(TokenKind::Word));
        assert!(filter.contains(TokenKind::Number));
        assert!(!filter.contains(TokenKind::Symbol));
        assert!(!filter.is_empty());
    }

    fn naive_next(text: &[u8], at: usize, filter: TokenFilter, mode: JumpMode) -> usize {
        let at = at.min(text.len());
        if filter.is_empty() {
            return at;
        }
        for (start, end) in runs(text, filter) {
            if at >= start && at < end {
                match mode {
                    JumpMode::FinishCurrent => return end,
                    JumpMode::NextStart => continue,
                }
            }
            if start >= at {
                return match mode {
                    JumpMode::FinishCurrent => end,
                    JumpMode::NextStart => start,
                };
            }
        }
        text.len()
    }

    fn naive_prev(text: &[u8], at: usize, filter: TokenFilter, mode: JumpMode) -> usize {
        let at = at.min(text.len());
        if filter.is_empty() {
            return at;
        }
        let mut previous = 0;
        for (start, end) in runs(text, filter) {
            if at > start && at <= end {
                return match mode {
                    JumpMode::FinishCurrent => start,
                    JumpMode::NextStart => previous,
                };
            }
            if end <= at {
                previous = start;
            } else if start >= at {
                break;
            }
        }
        previous
    }

    fn check_all_positions(text: &[u8]) {
        let filters = [
            TokenFilter::word(),
            TokenFilter::all(),
            TokenFilter::word().union(TokenFilter::symbol()),
            TokenFilter::default(),
        ];
        let modes = [JumpMode::FinishCurrent, JumpMode::NextStart];
        for at in 0..=text.len() {
            for filter in filters {
                for mode in modes {
                    assert_eq!(
                        next(text, at, filter, mode),
                        naive_next(text, at, filter, mode),
                        "вперёд: {text:?}, позиция {at}, {filter:?}, {mode:?}"
                    );
                    assert_eq!(
                        prev(text, at, filter, mode),
                        naive_prev(text, at, filter, mode),
                        "назад: {text:?}, позиция {at}, {filter:?}, {mode:?}"
                    );
                }
            }
        }
    }

    fn boundaries(text: &[u8]) -> Vec<usize> {
        let mut out = vec![0];
        let mut at = 0;
        while at < text.len() {
            at = grapheme::next(text, at);
            out.push(at);
        }
        out
    }

    fn check_boundaries(text: &[u8]) {
        let bounds = boundaries(text);
        let filters = [
            TokenFilter::word(),
            TokenFilter::number(),
            TokenFilter::symbol(),
            TokenFilter::all(),
            TokenFilter::word().union(TokenFilter::symbol()),
            TokenFilter::default(),
        ];
        let modes = [JumpMode::FinishCurrent, JumpMode::NextStart];
        for at in 0..=text.len() {
            for filter in filters {
                if filter.is_empty() {
                    continue;
                }
                for mode in modes {
                    let right = next(text, at, filter, mode);
                    assert!(
                        bounds.contains(&right),
                        "вперёд попали внутрь байта: {text:?}, {at}, {filter:?}, {mode:?} -> {right}"
                    );
                    let left = prev(text, at, filter, mode);
                    assert!(
                        bounds.contains(&left),
                        "назад попали внутрь байта: {text:?}, {at}, {filter:?}, {mode:?} -> {left}"
                    );
                }
            }
        }
    }

    #[test]
    fn jumps_stay_on_grapheme_boundaries() {
        check_boundaries(b"foo_bar baz123");
        check_boundaries("привет_мир 42".as_bytes());
        check_boundaries("e\u{301}x!".as_bytes());
        check_boundaries("👨‍👩‍👧‍👦! a".as_bytes());
        check_boundaries(b"a\r\nb");
        check_boundaries(b"a\xFFb");
        check_boundaries(format!("e{}x", "\u{301}".repeat(100)).as_bytes());
    }

    #[test]
    fn local_walks_match_a_full_scan() {
        check_all_positions(b"foo_bar baz123");
        check_all_positions(b"  foo\tbar\n");
        check_all_positions("привет_мир 42".as_bytes());
        check_all_positions("e\u{301}x!".as_bytes());
        check_all_positions("👨‍👩‍👧‍👦! a".as_bytes());
        check_all_positions(b"a\r\nb");
        check_all_positions(b"a\xFFb");
        check_all_positions(b"   ");
        check_all_positions(b"");
        let long_grapheme = format!("e{}x", "\u{301}".repeat(100));
        check_all_positions(long_grapheme.as_bytes());
    }
}

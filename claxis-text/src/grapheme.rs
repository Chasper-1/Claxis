//! Границы графем в байтах.
//!
//! Графема — то, что человек видит одним символом: буква с комбинирующим
//! знаком, эмодзи из нескольких кодовых точек, флаг из пары региональных
//! индикаторов. Терминал рисует графемы, курсор ходит по ним, а не по байтам
//! и не по кодовым точкам.
//!
//! Функции работают с куском текста, обычно со строкой. Документ целиком не
//! нужен: границы локальны, а хранить их между вызовами незачем.

use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

/// Размер первого окна для локальной проверки UTF-8.
///
/// Хватает на обычную букву с парой комбинирующихся знаков. Дальше окно
/// растёт, пока графема в него не влезет целиком.
const FIRST_WINDOW: usize = 16;

/// Начало следующей графемы после `at`.
///
/// `at` — байт на границе графемы. Внутри многобайтовой последовательности
/// функция всё равно не паникует: такой байт считается отдельной графемой,
/// чтобы битый текст не останавливал курсор.
///
/// UTF-8 проверяется только в окне вокруг `at`, а не во всём остатке строки:
/// время шага зависит от длины графемы, но не от длины строки.
pub fn next(text: &[u8], at: usize) -> usize {
    let at = at.min(text.len());
    if at >= text.len() {
        return text.len();
    }
    // `\r\n` — одна графема по правилам Unicode.
    if text[at] == b'\r' && text.get(at + 1) == Some(&b'\n') {
        return at + 2;
    }
    // ASCII без соседнего не-ASCII байта не с чем склеиваться.
    if text[at].is_ascii() && text.get(at + 1).is_none_or(|next| next.is_ascii()) {
        return at + 1;
    }
    forward_in_window(text, at)
}

/// Начало предыдущей графемы перед `at`.
///
/// Обратные правила склейки сложнее прямых, поэтому окно растёт назад, пока
/// `GraphemeCursor` не найдёт границу. Курсор знает полную длину текста и
/// начало окна, поэтому не принимает за границу место, где просто кончился
/// кусок. `at` не обязана быть границей — тогда возвращается граница слева.
pub fn prev(text: &[u8], at: usize) -> usize {
    let len = text.len();
    let at = at.min(len);
    if at == 0 {
        return 0;
    }
    // ASCII перед ASCII: предыдущая графема ровно в одном байте.
    let after_crlf = at >= 2 && text[at - 2] == b'\r' && text[at - 1] == b'\n';
    if text[at - 1].is_ascii()
        && !after_crlf
        && !(text[at - 1] == b'\r' && text.get(at) == Some(&b'\n'))
        && text.get(at).is_none_or(|next| next.is_ascii())
    {
        return at - 1;
    }
    // Если `at` попала внутрь символа, идём от начала следующего символа.
    let mut end = at;
    while end < len && text[end] & 0b1100_0000 == 0b1000_0000 {
        end += 1;
    }
    let mut span = FIRST_WINDOW;
    loop {
        let mut begin = end.saturating_sub(span);
        while begin > 0 && text[begin] & 0b1100_0000 == 0b1000_0000 {
            begin -= 1;
        }
        let Ok(part) = std::str::from_utf8(&text[begin..end]) else {
            return scan_back_from_start(text, at);
        };
        let mut cursor = GraphemeCursor::new(end, len, true);
        match cursor.prev_boundary(part, begin) {
            Ok(Some(previous)) => {
                if previous > begin {
                    return previous;
                }
                if begin == 0 {
                    return 0;
                }
                span = span.saturating_mul(2);
            }
            Ok(None) => {
                if begin == 0 {
                    return 0;
                }
                span = span.saturating_mul(2);
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                if begin == 0 {
                    return 0;
                }
                span = span.saturating_mul(2);
            }
            Err(GraphemeIncomplete::PreContext(need)) => {
                if need == 0 {
                    return 0;
                }
                span = (end - need).max(span.saturating_mul(2));
            }
            Err(_) => return scan_back_from_start(text, at),
        }
    }
}

/// Конец первой графемы в растущем окне.
fn forward_in_window(text: &[u8], at: usize) -> usize {
    let len = text.len();
    let mut span = FIRST_WINDOW;
    loop {
        let end = (at + span).min(len);
        match std::str::from_utf8(&text[at..end]) {
            Ok(valid) => {
                if let Some(grapheme) = valid.graphemes(true).next() {
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
                } else if let Ok(prefix) = std::str::from_utf8(&text[at..at + valid])
                    && let Some(grapheme) = prefix.graphemes(true).next()
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

/// Полный обход назад — только когда локальное окно встретило битый UTF-8.
fn scan_back_from_start(text: &[u8], at: usize) -> usize {
    let mut start = 0;
    loop {
        let end = next(text, start);
        if end >= at {
            return start;
        }
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::{next, prev};

    /// Старый полный обход вперёд: проверяет весь остаток строки.
    fn full_next(text: &[u8], at: usize) -> usize {
        let at = at.min(text.len());
        if at >= text.len() {
            return text.len();
        }
        if text[at] == b'\r' && text.get(at + 1) == Some(&b'\n') {
            return at + 2;
        }
        if text[at].is_ascii() && text.get(at + 1).is_none_or(|next| next.is_ascii()) {
            return at + 1;
        }
        let rest = &text[at..];
        let valid = match std::str::from_utf8(rest) {
            Ok(text) => text,
            Err(error) if error.valid_up_to() == 0 => return at + 1,
            Err(error) => match std::str::from_utf8(&rest[..error.valid_up_to()]) {
                Ok(text) => text,
                Err(_) => return at + 1,
            },
        };
        match valid.graphemes(true).next() {
            Some(grapheme) => at + grapheme.len(),
            None => at + 1,
        }
    }

    /// Старый полный обход назад: идёт от начала куска.
    fn full_prev(text: &[u8], at: usize) -> usize {
        let at = at.min(text.len());
        let mut start = 0;
        loop {
            let end = full_next(text, start);
            if end >= at {
                return start;
            }
            start = end;
        }
    }

    fn boundaries(text: &str) -> Vec<usize> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            out.push(at);
            at = next(bytes, at);
        }
        out.push(bytes.len());
        out
    }

    #[test]
    fn ascii_is_one_byte_per_grapheme() {
        let text = "hello";
        assert_eq!(boundaries(text), [0, 1, 2, 3, 4, 5]);
        assert_eq!(next(text.as_bytes(), 0), 1);
        assert_eq!(prev(text.as_bytes(), 5), 4);
    }

    #[test]
    fn a_combining_mark_stays_with_its_letter() {
        // e + U+0301: два кодовых точки, одна графема.
        let text = "e\u{301}x";
        assert_eq!(boundaries(text), [0, 3, 4]);
        assert_eq!(next(text.as_bytes(), 0), 3);
        assert_eq!(prev(text.as_bytes(), 3), 0);
    }

    #[test]
    fn an_emoji_zwj_sequence_is_one_grapheme() {
        let text = "👨‍👩‍👧‍👦";
        assert_eq!(boundaries(text), [0, text.len()]);
    }

    #[test]
    fn a_flag_is_one_grapheme() {
        let text = "🇷🇺";
        assert_eq!(boundaries(text), [0, text.len()]);
    }

    #[test]
    fn hangul_jamo_are_glued() {
        let text = "\u{1100}\u{1161}\u{11A8}";
        assert_eq!(boundaries(text), [0, text.len()]);
    }

    #[test]
    fn crlf_is_one_grapheme() {
        let text = "a\r\nb";
        assert_eq!(boundaries(text), [0, 1, 3, 4]);
        assert_eq!(prev(text.as_bytes(), 3), 1);
    }

    #[test]
    fn an_invalid_byte_does_not_stop_the_cursor() {
        let text = b"a\xFFb";
        assert_eq!(next(text, 0), 1);
        assert_eq!(next(text, 1), 2);
        assert_eq!(next(text, 2), 3);
        assert_eq!(prev(text, 3), 2);
        assert_eq!(prev(text, 2), 1);
    }

    #[test]
    fn a_position_inside_a_character_snaps_left() {
        let text = "ж";
        assert_eq!(prev(text.as_bytes(), 1), 0);
        assert_eq!(next(text.as_bytes(), 1), text.len());
    }

    #[test]
    fn the_end_is_the_end() {
        let text = "abc";
        assert_eq!(next(text.as_bytes(), 3), 3);
        assert_eq!(next(text.as_bytes(), 99), 3);
        assert_eq!(prev(text.as_bytes(), 0), 0);
        assert_eq!(prev(text.as_bytes(), 99), 2);
    }

    #[test]
    fn walking_forward_and_back_returns_to_the_same_place() {
        for text in [
            "hello",
            "привет",
            "e\u{301}x",
            "👨‍👩‍👧‍👦!",
            "🇷🇺🇺🇸",
            "\u{1100}\u{1161}\u{11A8}",
            "a\r\nb",
        ] {
            let bytes = text.as_bytes();
            let mut at = 0;
            while at < bytes.len() {
                let end = next(bytes, at);
                assert_eq!(prev(bytes, end), at, "не вернулись из {text:?} на {at}");
                at = end;
            }
        }
    }

    #[test]
    fn prev_sees_context_before_the_window() {
        // Окно начинается внутри первой пары флагов. Если курсор не знает
        // начало куска, он принимает за границу середину пары и отдаёт 29.
        let text = "aaaaaaaaaaaaaaaaa🇺🇸🇷🇸🇮";
        assert_eq!(prev(text.as_bytes(), 34), 33);
        assert_eq!(prev(text.as_bytes(), 35), 33);
    }

    #[test]
    fn local_walk_matches_a_full_scan() {
        let long_marks = format!("e{}x", "\u{301}".repeat(100));
        let long_zwj = "👨‍👩‍👧‍👦".repeat(8);
        let long_flags = "🇷".repeat(20);
        for text in [
            "hello",
            "привет",
            "e\u{301}x",
            "👨‍👩‍👧‍👦!",
            "🇷🇺🇺🇸",
            "aaaaaaaaaaaaaaaaa🇺🇸🇷🇸🇮",
            "\u{1100}\u{1161}\u{11A8}",
            "a\r\nb",
            "a\r\n\r\nb",
            long_marks.as_str(),
            long_zwj.as_str(),
            long_flags.as_str(),
        ] {
            let bytes = text.as_bytes();
            for at in 0..=bytes.len() {
                assert_eq!(
                    next(bytes, at),
                    full_next(bytes, at),
                    "вперёд: {text:?}, позиция {at}"
                );
                assert_eq!(
                    prev(bytes, at),
                    full_prev(bytes, at),
                    "назад: {text:?}, позиция {at}"
                );
            }
        }

        let broken = b"a\xFFb";
        for at in 0..=broken.len() {
            assert_eq!(
                next(broken, at),
                full_next(broken, at),
                "битый вперёд: {at}"
            );
            assert_eq!(prev(broken, at), full_prev(broken, at), "битый назад: {at}");
        }
    }
}

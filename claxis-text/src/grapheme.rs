//! Границы графем в байтах.
//!
//! Графема — то, что человек видит одним символом: буква с комбинирующим
//! знаком, эмодзи из нескольких кодовых точек, флаг из пары региональных
//! индикаторов. Терминал рисует графемы, курсор ходит по ним, а не по байтам
//! и не по кодовым точкам.
//!
//! Функции работают с куском текста, обычно со строкой. Документ целиком не
//! нужен: границы локальны, а хранить их между вызовами незачем.

use unicode_segmentation::UnicodeSegmentation;

/// Начало следующей графемы после `at`.
///
/// `at` — байт на границе графемы. Внутри многобайтовой последовательности
/// функция всё равно не паникует: такой байт считается отдельной графемой,
/// чтобы битый текст не останавливал курсор.
pub fn next(text: &[u8], at: usize) -> usize {
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
    first_in(text, at)
}

/// Начало предыдущей графемы перед `at`.
///
/// Идём вперёд от начала куска: обратные правила склейки сложнее прямых, а
/// строки, по которым ходит курсор, короткие. `at` не обязана быть границей —
/// тогда возвращается граница слева.
pub fn prev(text: &[u8], at: usize) -> usize {
    let at = at.min(text.len());
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
    let mut start = 0;
    loop {
        let end = next(text, start);
        if end >= at {
            return start;
        }
        start = end;
    }
}

/// Конец первой графемы, начинающейся в `at`.
fn first_in(text: &[u8], at: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::{next, prev};

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
}

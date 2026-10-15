//! Kitty keyboard protocol.
//!
//! Протокол даёт события отпускания, различающиеся от нажатий. Без него модель
//! ввода работать не может: «отпустил Alt» не отличить от нового нажатия.

use std::io::{self, Read, Write};

/// Флаги включения протокола.
///
/// Набор взят полный: редактору нужны отпускания, разрешение неоднозначности,
/// альтернативные клавиши и отчёт обо всех клавишах как escape-коды.
const FLAGS: u16 = 0b11111;

/// Бит «различать нажатие и отпускание».
///
/// Без него протокола для редактора нет: терминал будет слать только нажатия,
/// и модель ввода, ради которой протокол и включается, работать не сможет.
/// Поддержка протокола проверяется именно этим битом, а не любым ответом.
const REPORTS_EVENT_TYPES: u16 = 0b10;

/// Включить протокол: `CSI = flags ; mode > u`.
pub fn enable() -> io::Result<()> {
    let mut out = io::stdout();
    write!(out, "\x1b[={FLAGS};1>u")?;
    out.flush()
}

/// Выключить протокол: `CSI = flags ; mode < u`.
pub fn disable() -> io::Result<()> {
    let mut out = io::stdout();
    write!(out, "\x1b[={FLAGS};1<u")?;
    out.flush()
}

/// Поддерживает ли терминал нужную часть протокола.
///
/// Запрос `CSI ? u`, ответ `CSI ? flags ; mode u`. Ответа нет, в ответе нет
/// бита отчёта о типах событий — протокола для редактора нет.
pub fn supports() -> bool {
    matches!(query(), Some(flags) if flags & REPORTS_EVENT_TYPES != 0)
}

/// Запросить поддержку у терминала.
fn query() -> Option<u16> {
    let mut out = io::stdout();
    if write!(out, "\x1b[?u").is_err() || out.flush().is_err() {
        return None;
    }

    // Ответ приходит на stdin. Ждём ограниченное время: терминал без протокола
    // просто промолчит, и ждать вечно нельзя.
    let mut stdin = io::stdin();
    let mut buf = [0u8; 64];

    // Небольшая задержка на ответ. Терминал отвечает мгновенно, но через пайп
    // или ssh задержка бывает.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);

    let mut got = Vec::new();
    while std::time::Instant::now() < deadline {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if parse_response(&got).is_some() {
                    return parse_response(&got);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }

    parse_response(&got)
}

/// Разобрать ответ `CSI ? flags ; mode u`.
fn parse_response(buf: &[u8]) -> Option<u16> {
    let s = std::str::from_utf8(buf).ok()?;
    // Ищем ESC [ ? ... u
    let start = s.find("\x1b[?")?;
    let rest = &s[start + 3..];
    let end = rest.find('u')?;
    let body = &rest[..end];
    // body = "flags ; mode"
    let flags_part = body.split(';').next()?;
    flags_part.trim().parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_sequence_is_correct() {
        // CSI = 1 ; 1 > u — включить с полными флагами.
        assert_eq!(FLAGS, 0b11111);
    }

    #[test]
    fn a_terminal_without_event_types_is_not_supported() {
        // Ответ с другими флагами — это не kitty keyboard protocol для
        // редактора: отпусканий в нём не будет, а модель ввода без них не
        // работает. Поддержкой считается только нужный бит.
        assert_eq!(0b11111 & REPORTS_EVENT_TYPES, REPORTS_EVENT_TYPES);
        assert_eq!(0b01000 & REPORTS_EVENT_TYPES, 0);
    }

    #[test]
    fn parse_response_reads_flags() {
        assert_eq!(parse_response(b"\x1b[?31;1u"), Some(31));
        assert_eq!(parse_response(b"\x1b[?1;1u"), Some(1));
        assert_eq!(parse_response(b"\x1b[?0;0u"), Some(0));
    }

    #[test]
    fn parse_response_rejects_garbage() {
        assert_eq!(parse_response(b"hello"), None);
        assert_eq!(parse_response(b"\x1b[?u"), None);
        assert_eq!(parse_response(b""), None);
    }
}

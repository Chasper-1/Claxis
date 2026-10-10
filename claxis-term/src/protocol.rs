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

/// Поддерживает ли терминал протокол.
///
/// Запрос `CSI ? u`, ответ `CSI ? flags ; mode u`. Если ответа нет или флагов
/// нет — протокол не поддерживается.
pub fn supports() -> bool {
    query().is_some()
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

//! Kitty keyboard protocol.
//!
//! Протокол даёт события отпускания, различающиеся от нажатий. Без него модель
//! ввода работать не может: «отпустил Alt» не отличить от нового нажатия.

use std::io::{self, Read, Write};

/// Флаги включения протокола.
///
/// Полный набор (`0b11111`): отпускания, разрешение неоднозначности,
/// альтернативные клавиши и отчёт обо всех клавишах как escape-коды. Бит
/// `0b10` — «различать нажатие и отпускание»: без него терминал шлёт только
/// нажатия, и модель ввода не работает.
const FLAGS: u16 = 0b11111;

/// Запрос поддержки: `CSI ? u`.
fn query_sequence() -> String {
    "\x1b[?u".to_string()
}

/// Включение протокола: `CSI = flags ; mode > u`.
fn enable_sequence() -> String {
    format!("\x1b[={FLAGS};1>u")
}

/// Выключение протокола: `CSI = flags ; mode < u`.
fn disable_sequence() -> String {
    format!("\x1b[={FLAGS};1<u")
}

/// Включить протокол.
pub fn enable() -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(enable_sequence().as_bytes())?;
    out.flush()
}

/// Выключить протокол.
pub fn disable() -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(disable_sequence().as_bytes())?;
    out.flush()
}

/// Поддерживает ли терминал протокол.
///
/// Запрос `CSI ? u`, ответ `CSI ? flags ; mode u`. Ответ — и есть признак
/// поддержки: число в нём показывает, какие улучшения сейчас **включены**, а
/// на чистом терминале их ноль. Проверять биты в ответе нельзя: терминал с
/// полной поддержкой отвечает `0`, и редактор отказался бы работать впустую.
pub fn supports() -> bool {
    match ask() {
        Some(answer) => supported(&answer),
        None => false,
    }
}

/// Ответ терминала без ответа: поддержки нет.
fn supported(answer: &[u8]) -> bool {
    parse_response(answer).is_some()
}

/// Запросить у терминала и вернуть ответ целиком.
///
/// Ответ может прийти не сразу и вместе с мусором от предыдущего ввода, поэтому
/// собираем всё, что пришло за время ожидания, и разбираем как единый поток.
fn ask() -> Option<Vec<u8>> {
    let mut out = io::stdout();
    out.write_all(query_sequence().as_bytes()).ok()?;
    out.flush().ok()?;

    // Терминал отвечает мгновенно, но через псевдотерминал или ssh задержка
    // бывает. Ждать вечно нельзя: терминал без протокола просто промолчит, и
    // редактор должен сказать об этом и выйти, а не висеть.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    let mut stdin = io::stdin();
    let mut buf = [0u8; 64];
    let mut got: Vec<u8> = Vec::new();

    while std::time::Instant::now() < deadline {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if parse_response(&got).is_some() {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Любая другая ошибка чтения означает одно: ответа не будет. Ждать
            // дальше бессмысленно, а пользователь увидит `NoKittyProtocol` с
            // внятным объяснением — молчания тут нет.
            Err(_) => break,
        }
    }

    if got.is_empty() { None } else { Some(got) }
}

/// Разобрать ответ `CSI ? flags ; mode u`.
fn parse_response(buf: &[u8]) -> Option<u16> {
    let answer = std::str::from_utf8(buf).ok()?;
    let start = answer.find("\x1b[?")?;
    let rest = &answer[start + 3..];
    let end = rest.find('u')?;
    let body = &rest[..end];
    // body = "flags ; mode"
    let flags_part = body.split(';').next()?;
    flags_part.trim().parse::<u16>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ответ терминала, который знает протокол, но ещё ничего не включил.
    /// Так отвечает kitty, foot, Ghostty и WezTerm на чистом запуске.
    const REPLY_ZERO: &[u8] = b"\x1b[?0;1u";

    /// Ответ терминала с уже включёнными улучшениями.
    const REPLY_ACTIVE: &[u8] = b"\x1b[?31;1u";

    #[test]
    fn query_sequence_is_the_one_from_the_spec() {
        assert_eq!(query_sequence(), "\x1b[?u");
    }

    #[test]
    fn enable_sequence_carries_every_flag() {
        // Бит 0b10 обязан быть во включении: без отпусканий модель ввода мертва.
        assert_eq!(enable_sequence(), "\x1b[=31;1>u");
    }

    #[test]
    fn disable_sequence_is_the_mirror_of_enable() {
        assert_eq!(disable_sequence(), "\x1b[=31;1<u");
    }

    #[test]
    fn a_clean_terminal_reply_means_supported() {
        // Именно этот случай ломал проверку: ответ с нулём в флагах — это
        // поддержка, а не её отсутствие.
        assert!(supported(REPLY_ZERO));
        assert_eq!(parse_response(REPLY_ZERO), Some(0));
    }

    #[test]
    fn a_reply_with_active_flags_means_supported() {
        assert!(supported(REPLY_ACTIVE));
        assert_eq!(parse_response(REPLY_ACTIVE), Some(31));
    }

    #[test]
    fn every_flag_value_means_supported() {
        // Поддержка не зависит от того, что терминал уже включил.
        for flags in 0u16..=31 {
            let reply = format!("\x1b[?{flags};1u");
            assert!(supported(reply.as_bytes()), "flags {flags}");
        }
    }

    #[test]
    fn silence_means_unsupported() {
        assert!(!supported(b""));
        assert!(!supported(
            "текст, который человек нажал до запроса".as_bytes()
        ));
    }

    #[test]
    fn a_truncated_reply_means_unsupported() {
        // Терминал не дописал ответ: ждать дальше незачем, но и поддержку
        // объявлять рано — такого ответа от живого терминала не бывает.
        assert_eq!(parse_response(b"\x1b[?"), None);
        assert_eq!(parse_response(b"\x1b[?0;1"), None);
        assert_eq!(parse_response(b"\x1b[?abc;1u"), None);
    }

    #[test]
    fn a_reply_after_other_input_is_still_found() {
        // Человек успел нажать что-то раньше: ответ приходит после мусора.
        let mut stream = Vec::new();
        stream.extend_from_slice("привет\x1b[A".as_bytes());
        stream.extend_from_slice(REPLY_ZERO);
        assert!(supported(&stream));
    }

    #[test]
    fn the_first_reply_wins_when_the_stream_has_several() {
        // Несколько ответов подряд — терминал ответил и на что-то ещё.
        let mut stream = Vec::new();
        stream.extend_from_slice(REPLY_ZERO);
        stream.extend_from_slice(REPLY_ACTIVE);
        assert_eq!(parse_response(&stream), Some(0));
    }

    #[test]
    fn a_reply_split_across_reads_is_still_parsed() {
        // Ответ может прийти двумя кусками: первый обрыв, второй остаток.
        let mut stream = Vec::new();
        stream.extend_from_slice(b"\x1b[?");
        assert_eq!(parse_response(&stream), None);
        stream.extend_from_slice(b"0;1u");
        assert!(supported(&stream));
    }

    #[test]
    fn broken_utf8_means_unsupported() {
        assert_eq!(parse_response(&[0xff, 0xfe, b'u']), None);
    }
}

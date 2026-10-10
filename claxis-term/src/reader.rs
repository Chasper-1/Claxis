//! Чтение событий терминала.
//!
//! Парсинг escape-последовательностей — забота crossterm. Здесь только
//! конвертация в свои типы: нажатие и отпускание различимы, а у crossterm
//! для этого нет отдельного типа.

use std::io;
use std::time::Instant;

use crossterm::event::{Event as CEvent, KeyCode as CKeyCode, KeyEventKind, KeyModifiers};

use crate::event::{Event, Key, KeyCode, Modifiers};

/// Следующее событие от терминала.
pub fn next() -> io::Result<Event> {
    loop {
        let cevent = crossterm::event::read()?;
        if let Some(event) = convert(cevent) {
            return Ok(event);
        }
        // События, которые редактор не разобрал, пропускаем: они не должны
        // блокировать чтение.
    }
}

/// Конвертировать событие crossterm в своё.
fn convert(cevent: CEvent) -> Option<Event> {
    match cevent {
        CEvent::Key(key) => {
            let kind = key.kind;
            let code = convert_key_code(key.code)?;
            let modifiers = convert_modifiers(key.modifiers);
            let at = Instant::now();
            let key = Key { code, modifiers };
            match kind {
                KeyEventKind::Press | KeyEventKind::Repeat => Some(Event::KeyPressed { key, at }),
                KeyEventKind::Release => Some(Event::KeyReleased { key, at }),
            }
        }
        CEvent::Resize(cols, rows) => Some(Event::Resized { cols, rows }),
        CEvent::FocusGained => Some(Event::FocusGained),
        CEvent::FocusLost => Some(Event::FocusLost),
        // Мышь и вставка из буфера пока не разобраны.
        CEvent::Mouse(_) | CEvent::Paste(_) => Some(Event::Unknown),
    }
}

/// Конвертировать код клавиши.
fn convert_key_code(code: CKeyCode) -> Option<KeyCode> {
    Some(match code {
        CKeyCode::Char(c) => KeyCode::Char(c),
        CKeyCode::Enter => KeyCode::Enter,
        CKeyCode::Esc => KeyCode::Escape,
        CKeyCode::Backspace => KeyCode::Backspace,
        CKeyCode::Tab => KeyCode::Tab,
        CKeyCode::Up => KeyCode::Up,
        CKeyCode::Down => KeyCode::Down,
        CKeyCode::Left => KeyCode::Left,
        CKeyCode::Right => KeyCode::Right,
        CKeyCode::Delete => KeyCode::Delete,
        CKeyCode::Home => KeyCode::Home,
        CKeyCode::End => KeyCode::End,
        CKeyCode::PageUp => KeyCode::PageUp,
        CKeyCode::PageDown => KeyCode::PageDown,
        CKeyCode::F(n) => KeyCode::F(n),
        // Служебные клавиши без смысла для редактора.
        _ => return None,
    })
}

/// Конвертировать модификаторы.
fn convert_modifiers(mods: KeyModifiers) -> Modifiers {
    Modifiers {
        ctrl: mods.contains(KeyModifiers::CONTROL),
        alt: mods.contains(KeyModifiers::ALT),
        shift: mods.contains(KeyModifiers::SHIFT),
        super_: mods.contains(KeyModifiers::SUPER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_becomes_key_pressed() {
        let cevent = CEvent::Key(crossterm::event::KeyEvent::new(
            CKeyCode::Char('a'),
            KeyModifiers::NONE,
        ));
        // KeyEvent::new по умолчанию Press.
        match convert(cevent) {
            Some(Event::KeyPressed { key, .. }) => {
                assert_eq!(key.code, KeyCode::Char('a'));
                assert_eq!(key.modifiers, Modifiers::NONE);
            }
            other => panic!("ожидалось KeyPressed, получено {other:?}"),
        }
    }

    #[test]
    fn release_becomes_key_released() {
        let cevent = CEvent::Key(crossterm::event::KeyEvent::new_with_kind(
            CKeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        match convert(cevent) {
            Some(Event::KeyReleased { key, .. }) => {
                assert_eq!(key.code, KeyCode::Char('a'));
            }
            other => panic!("ожидалось KeyReleased, получено {other:?}"),
        }
    }

    #[test]
    fn modifiers_are_converted() {
        let cevent = CEvent::Key(crossterm::event::KeyEvent::new(
            CKeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        match convert(cevent) {
            Some(Event::KeyPressed { key, .. }) => {
                assert!(key.modifiers.ctrl);
                assert!(key.modifiers.alt);
                assert!(!key.modifiers.shift);
            }
            other => panic!("ожидалось KeyPressed, получено {other:?}"),
        }
    }

    #[test]
    fn resize_is_converted() {
        match convert(CEvent::Resize(100, 40)) {
            Some(Event::Resized { cols, rows }) => {
                assert_eq!(cols, 100);
                assert_eq!(rows, 40);
            }
            other => panic!("ожидалось Resized, получено {other:?}"),
        }
    }

    #[test]
    fn unknown_keys_are_skipped() {
        // CapsLock не имеет смысла для редактора — пропускаем.
        let cevent = CEvent::Key(crossterm::event::KeyEvent::new(
            CKeyCode::CapsLock,
            KeyModifiers::NONE,
        ));
        assert_eq!(convert(cevent), None);
    }

    #[test]
    fn mouse_is_unknown() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let cevent = CEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(convert(cevent), Some(Event::Unknown));
    }
}

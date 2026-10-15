//! Чтение событий терминала.
//!
//! Парсинг escape-последовательностей — забота crossterm. Здесь только
//! конвертация в свои типы: нажатие и отпускание различимы, а у crossterm
//! для этого нет отдельного типа.

use std::io;
use std::time::Instant;

use crossterm::event::{Event as CEvent, KeyCode as CKeyCode, KeyEventKind, KeyModifiers};

use crate::event::{Event, Key, KeyCode, Modifiers, MouseButton};

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
                KeyEventKind::Press => Some(Event::KeyPressed { key, at }),
                KeyEventKind::Release => Some(Event::KeyReleased { key, at }),
                KeyEventKind::Repeat => Some(Event::KeyRepeated { key, at }),
            }
        }
        CEvent::Resize(cols, rows) => Some(Event::Resized { cols, rows }),
        CEvent::FocusGained => Some(Event::FocusGained),
        CEvent::FocusLost => Some(Event::FocusLost),
        // Вставка и мышь терять нельзя: молча пропавший текст хуже отшибка.
        CEvent::Paste(text) => Some(Event::Paste { text }),
        CEvent::Mouse(m) => convert_mouse(m),
    }
}

/// Конвертировать мышь. Движения без нажатия пока не нужны редактору: курсор
/// двигают клавиши, а выделение мышью появится вместе с режимом выделения.
fn convert_mouse(m: crossterm::event::MouseEvent) -> Option<Event> {
    let button = match m.kind {
        crossterm::event::MouseEventKind::Down(b) => convert_button(b),
        crossterm::event::MouseEventKind::Up(b) => convert_button(b),
        _ => return None,
    }?;
    Some(Event::Mouse {
        button,
        col: m.column,
        row: m.row,
    })
}

/// Конвертировать кнопку мыши.
///
/// Кнопок у мыши больше трёх, но редактору нужны эти: остальные в сюда не
/// попадают, потому что `crossterm` их не различает.
fn convert_button(b: crossterm::event::MouseButton) -> Option<MouseButton> {
    Some(match b {
        crossterm::event::MouseButton::Left => MouseButton::Left,
        crossterm::event::MouseButton::Right => MouseButton::Right,
        crossterm::event::MouseButton::Middle => MouseButton::Middle,
    })
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
    fn repeat_is_not_a_new_press() {
        // Автоповтор продолжает нажатие, а не начинает новое: иначе время
        // удержания перезаписывалось бы на каждом повторе.
        let cevent = CEvent::Key(crossterm::event::KeyEvent::new_with_kind(
            CKeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ));
        assert!(matches!(convert(cevent), Some(Event::KeyRepeated { .. })));
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
    fn a_mouse_press_keeps_its_button_and_place() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let cevent = CEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 7,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        match convert(cevent) {
            Some(Event::Mouse { button, col, row }) => {
                assert_eq!(button, MouseButton::Left);
                assert_eq!(col, 7);
                assert_eq!(row, 3);
            }
            other => panic!("ожидалось Mouse, получено {other:?}"),
        }
    }

    #[test]
    fn a_mouse_move_without_a_button_is_skipped() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let cevent = CEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(convert(cevent), None);
    }

    #[test]
    fn a_paste_keeps_its_text() {
        // Вставка теряться не может: молча пропавший текст хуже отшибка.
        let cevent = CEvent::Paste("вставленное".to_string());
        match convert(cevent) {
            Some(Event::Paste { text }) => assert_eq!(text, "вставленное"),
            other => panic!("ожидалось Paste, получено {other:?}"),
        }
    }
}

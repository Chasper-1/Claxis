//! Цикл редактора: события терминала идут в документ.
//!
//! Цикл тонкий. Он ничего не решает про смысл клавиш и ничего не рисует:
//! читает событие, отдаёт его документу, следит за временем. Решения принимает
//! документ, отрисовка появится позже отдельным слоем.
//!
//! Снапшоты по времени тикают здесь, а не в документе: часы редактора и часы
//! документа — разные вещи, и решение «пора» принимает тот, кто видит
//! терминал.

use std::collections::HashMap;
use std::io;
use std::time::Instant;

use crate::api::term::{Event, Key, KeyCode, Modifiers, Terminal};
use crate::router::session::Document;

/// Что цикл сделал с событием.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Работаем дальше.
    Continue,
    /// Из редактора выходим.
    Quit,
}

/// Цикл: держит нажатые клавиши и гоняет события в документ.
#[derive(Debug, Default)]
pub struct Cycle {
    /// Клавиши, сейчас нажатые: по ним считается, сколько нажатие пролежало.
    pressed: HashMap<Key, Instant>,
}

impl Cycle {
    /// Пустой цикл: ничего не нажато.
    pub fn new() -> Self {
        Self::default()
    }

    /// Обработать одно событие.
    ///
    /// Нажатие и отпускание идут в статистику документа: сколько нажатий было
    /// и сколько времени они заняли. Остальные события пока ничего не меняют.
    pub fn handle(&mut self, event: Event, doc: &mut Document) -> Action {
        match event {
            Event::KeyPressed { key, time } => {
                if key.code == KeyCode::Char('q') && key.modifiers == Modifiers::CTRL {
                    // Выход до учёта нажатия: время ушедшего нажатия уже ничего
                    // не значит, документ закрывается.
                    return Action::Quit;
                }
                // Клавиша уже нажата — значит это повтор без отпускания,
                // время отсчитывается от первого нажатия. Перезапись
                // отбросила бы уже накопленное удержание.
                match self.pressed.entry(key) {
                    std::collections::hash_map::Entry::Occupied(_) => {}
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(time);
                        doc.note_press();
                    }
                }
            }
            Event::KeyReleased { key, time } => {
                // Отпускание без нажатия — потерянное событие: времени у него
                // нет, и выдумывать ноль секунд вместо неизвестного неправильно.
                if let Some(pressed) = self.pressed.remove(&key) {
                    doc.note_held(time.saturating_duration_since(pressed));
                }
            }
            Event::Resized { .. } | Event::FocusGained | Event::FocusLost => {}
            // Автоповтор не трогает ни времени, ни счётчика: он продолжает
            // нажатие, а не начинает новое. Иначе зажатая клавиша считалась бы
            // десятками нажатий, а удержание — считалось бы от каждого
            // повтора заново.
            Event::KeyRepeated { .. } => {}
            // Вставка и мышь пока некуда деть: вставлять текст некуда без
            // каретки, выделять нечем. Событие названо явно, чтобы оно не
            // потерялось молча в общем «остальное».
            Event::Paste { .. } | Event::Mouse { .. } => {}
        }
        Action::Continue
    }

    /// Читать события, пока пользователь не вышел.
    ///
    /// Перед каждым событием проверяется конфиг и снимается снапшот, если
    /// прошло достаточно времени. Цикл стоит на вводе: пока событий нет,
    /// таймер не тикает — это цена блокирующего чтения, и за неё потом
    /// заплатим переходом на опрос с таймаутом.
    pub fn run(
        &mut self,
        term: &mut Terminal,
        editor: &mut crate::router::Editor,
        doc: &mut Document,
        messages: &dyn crate::messages::Messages,
        mut report: impl FnMut(&str),
    ) -> io::Result<()> {
        loop {
            // Изменение конфига приходит от системы. Пока цикл стоит на вводе,
            // новости ждут: разбудить его от файла — отдельная задача.
            if let Some(reloaded) = editor.poll_watch() {
                for issue in &reloaded.issues {
                    report(&issue.text());
                }
            }
            doc.snapshot_if_due();
            // Снапшот мог не записаться: об этом человек обязан знать, иначе
            // он спокойно закроет редактор и потеряет работу.
            if let Some(reason) = doc.snapshot_error() {
                let path = doc.path().display().to_string();
                report(&messages.snapshot_failed(&path, &reason));
            }

            let event = term.next_event()?;
            if self.handle(event, doc) == Action::Quit {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::term::Event;
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::router::session::Session;

    /// Момент в истории, из которого считается время нажатия.
    fn at(millis: u64) -> Instant {
        Instant::now() + Duration::from_millis(millis)
    }

    fn key(code: KeyCode) -> Key {
        Key {
            code,
            modifiers: Modifiers::NONE,
        }
    }

    fn press(code: KeyCode, at: Instant) -> Event {
        Event::KeyPressed {
            key: key(code),
            time: at,
        }
    }

    fn release(code: KeyCode, at: Instant) -> Event {
        Event::KeyReleased {
            key: key(code),
            time: at,
        }
    }

    /// Настоящий документ с настоящей сессией: цикл считает статистику.
    struct Fixture {
        dir: PathBuf,
        doc: Document,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-cycle-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("a.rs");
            std::fs::write(&file, "abc\n").unwrap();
            let doc = Session::open_at_with(&dir, 3, true, 64)
                .unwrap()
                .open_file(&file)
                .unwrap();
            Self { dir, doc }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_fresh_cycle_has_nothing_pressed() {
        let cycle = Cycle::new();
        assert!(cycle.pressed.is_empty());
    }

    #[test]
    fn ctrl_q_leaves_the_editor() {
        let mut fixture = Fixture::new("quit");
        let mut cycle = Cycle::new();
        let quit = Event::KeyPressed {
            key: Key {
                code: KeyCode::Char('q'),
                modifiers: Modifiers::CTRL,
            },
            time: Instant::now(),
        };
        assert_eq!(cycle.handle(quit, &mut fixture.doc), Action::Quit);
    }

    #[test]
    fn ctrl_q_is_not_counted_as_a_press() {
        // Нажатие, которым выходят, в статистику не идёт: работа кончилась,
        // считать там нечего.
        let mut fixture = Fixture::new("quit-not-counted");
        let mut cycle = Cycle::new();
        let before = fixture.doc.stats().presses;
        cycle.handle(
            Event::KeyPressed {
                key: Key {
                    code: KeyCode::Char('q'),
                    modifiers: Modifiers::CTRL,
                },
                time: Instant::now(),
            },
            &mut fixture.doc,
        );
        assert_eq!(fixture.doc.stats().presses, before);
        assert!(cycle.pressed.is_empty(), "выход не должен держать клавишу");
    }

    #[test]
    fn other_combinations_with_q_keep_working() {
        // Выход только у чистого Ctrl+Q: `q` без Ctrl и Ctrl с другой буквой
        // остаются обычными нажатиями, иначе выйти было бы нечем.
        let mut fixture = Fixture::new("other-q");
        let mut cycle = Cycle::new();
        let plain = press(KeyCode::Char('q'), Instant::now());
        assert_eq!(cycle.handle(plain, &mut fixture.doc), Action::Continue);

        let other = Event::KeyPressed {
            key: Key {
                code: KeyCode::Char('x'),
                modifiers: Modifiers::CTRL,
            },
            time: Instant::now(),
        };
        assert_eq!(cycle.handle(other, &mut fixture.doc), Action::Continue);
        assert_eq!(cycle.pressed.len(), 2);
    }

    #[test]
    fn ctrl_q_with_an_extra_modifier_does_not_quit() {
        // Ctrl+Shift+Q — это не то же самое, что Ctrl+Q: человек нажал
        // комбинацию, а не вышел.
        let mut fixture = Fixture::new("ctrl-shift-q");
        let mut cycle = Cycle::new();
        let event = Event::KeyPressed {
            key: Key {
                code: KeyCode::Char('q'),
                modifiers: Modifiers {
                    shift: true,
                    ..Modifiers::CTRL
                },
            },
            time: Instant::now(),
        };
        assert_eq!(cycle.handle(event, &mut fixture.doc), Action::Continue);
    }

    #[test]
    fn a_press_and_its_release_are_counted_once() {
        let mut fixture = Fixture::new("press-release");
        let mut cycle = Cycle::new();
        let start = at(0);
        cycle.handle(press(KeyCode::Char('a'), start), &mut fixture.doc);
        cycle.handle(release(KeyCode::Char('a'), at(120)), &mut fixture.doc);
        let stats = fixture.doc.stats();
        assert_eq!(stats.presses, 1);
        assert!(
            stats.pressed >= Duration::from_millis(100),
            "время нажатия не засчитано: {:?}",
            stats.pressed
        );
        assert!(cycle.pressed.is_empty(), "клавиша должна быть отпущена");
    }

    #[test]
    fn a_release_without_a_press_adds_no_time() {
        // Потерянное отпускание: времени у него нет, и выдумывать ноль
        // вместо неизвестного неправильно.
        let mut fixture = Fixture::new("release-alone");
        let mut cycle = Cycle::new();
        cycle.handle(release(KeyCode::Char('a'), at(50)), &mut fixture.doc);
        assert_eq!(fixture.doc.stats().presses, 0);
        assert_eq!(fixture.doc.stats().pressed, Duration::ZERO);
    }

    #[test]
    fn a_release_of_another_key_does_not_close_the_gap() {
        // Нажата `a`, отпущена `b`: время `b` не имеет и не должно вычитаться
        // из удержания `a`.
        let mut fixture = Fixture::new("wrong-release");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        cycle.handle(release(KeyCode::Char('b'), at(50)), &mut fixture.doc);
        assert_eq!(cycle.pressed.len(), 1, "`a` должна остаться нажатой");
        cycle.handle(release(KeyCode::Char('a'), at(100)), &mut fixture.doc);
        assert!(
            fixture.doc.stats().pressed >= Duration::from_millis(100),
            "время `a` посчитано не от её нажатия: {:?}",
            fixture.doc.stats().pressed
        );
    }

    #[test]
    fn two_keys_held_at_once_are_kept_apart() {
        let mut fixture = Fixture::new("two-keys");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        cycle.handle(press(KeyCode::Char('b'), at(10)), &mut fixture.doc);
        assert_eq!(cycle.pressed.len(), 2);
        cycle.handle(release(KeyCode::Char('a'), at(100)), &mut fixture.doc);
        assert_eq!(cycle.pressed.len(), 1);
        cycle.handle(release(KeyCode::Char('b'), at(110)), &mut fixture.doc);
        assert!(cycle.pressed.is_empty());
    }

    #[test]
    fn the_same_key_held_longer_keeps_the_first_press_time() {
        // Повтор без отпускания — тот же палец на клавише. Время отсчитывается
        // от первого нажатия, а не от последнего.
        let mut fixture = Fixture::new("repeat");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        cycle.handle(
            Event::KeyRepeated {
                key: key(KeyCode::Char('a')),
                time: at(50),
            },
            &mut fixture.doc,
        );
        cycle.handle(release(KeyCode::Char('a'), at(100)), &mut fixture.doc);
        assert_eq!(fixture.doc.stats().presses, 1, "повтор не новое нажатие");
        assert!(
            fixture.doc.stats().pressed >= Duration::from_millis(100),
            "время отсчитано от повтора: {:?}",
            fixture.doc.stats().pressed
        );
    }

    #[test]
    fn a_repeat_without_a_press_changes_nothing() {
        let mut fixture = Fixture::new("repeat-alone");
        let mut cycle = Cycle::new();
        cycle.handle(
            Event::KeyRepeated {
                key: key(KeyCode::Char('a')),
                time: at(10),
            },
            &mut fixture.doc,
        );
        assert!(cycle.pressed.is_empty());
        assert_eq!(fixture.doc.stats().presses, 0);
    }

    #[test]
    fn pressing_the_same_key_twice_without_release_keeps_the_first_time() {
        // Автоповтор приходит как `KeyPressed`, а не `KeyRepeated`: второй
        // настоящий нажатие той же клавиши не должен обнулять отсчёт.
        let mut fixture = Fixture::new("press-twice");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        cycle.handle(press(KeyCode::Char('a'), at(40)), &mut fixture.doc);
        assert_eq!(cycle.pressed.len(), 1);
        cycle.handle(release(KeyCode::Char('a'), at(90)), &mut fixture.doc);
        assert!(
            fixture.doc.stats().pressed >= Duration::from_millis(90),
            "отсчёт пошёл от второго нажатия: {:?}",
            fixture.doc.stats().pressed
        );
    }

    #[test]
    fn a_release_before_the_press_time_does_not_underflow() {
        // Часы могут разъехаться, и время выйдет отрицательным. Паникой это
        // быть не должно: время просто не засчитывается.
        let mut fixture = Fixture::new("backwards");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(100)), &mut fixture.doc);
        cycle.handle(release(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        assert_eq!(fixture.doc.stats().pressed, Duration::ZERO);
    }

    #[test]
    fn window_and_focus_events_change_nothing() {
        let mut fixture = Fixture::new("focus");
        let mut cycle = Cycle::new();
        let before = fixture.doc.stats();
        for event in [
            Event::Resized { cols: 80, rows: 24 },
            Event::FocusGained,
            Event::FocusLost,
        ] {
            assert_eq!(cycle.handle(event, &mut fixture.doc), Action::Continue);
        }
        let after = fixture.doc.stats();
        assert_eq!(after.presses, before.presses);
        assert_eq!(after.pressed, before.pressed);
        assert!(cycle.pressed.is_empty());
    }

    #[test]
    fn paste_and_mouse_do_not_touch_the_statistics() {
        // Событие названо явно, чтобы не потеряться молча: пока ему некуда
        // идти, оно не должно влиять на счётчики.
        let mut fixture = Fixture::new("paste");
        let mut cycle = Cycle::new();
        for event in [
            Event::Paste {
                text: "вставленный текст".to_string(),
            },
            Event::Mouse {
                button: crate::api::term::MouseButton::Left,
                col: 1,
                row: 2,
            },
        ] {
            assert_eq!(cycle.handle(event, &mut fixture.doc), Action::Continue);
        }
        assert_eq!(fixture.doc.stats().presses, 0);
        assert_eq!(fixture.doc.stats().input_chars, 0);
    }

    #[test]
    fn a_key_with_no_release_keeps_its_time_unfinished() {
        // Клавиша, которую так и не отпустили, не должна попадать в статистику
        // как удержание: время отпускания ещё не наступило.
        let mut fixture = Fixture::new("no-release");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        assert_eq!(cycle.pressed.len(), 1);
        assert_eq!(fixture.doc.stats().pressed, Duration::ZERO);
        assert_eq!(fixture.doc.stats().presses, 1);
    }

    #[test]
    fn handles_keep_working_after_a_quit_was_asked() {
        // Решение «выйти» не ломает само состояние: следующий вызов всё ещё
        // считает, а не паникует на полуразобранном нажатии.
        let mut fixture = Fixture::new("after-quit");
        let mut cycle = Cycle::new();
        cycle.handle(press(KeyCode::Char('a'), at(0)), &mut fixture.doc);
        let quit = Event::KeyPressed {
            key: Key {
                code: KeyCode::Char('q'),
                modifiers: Modifiers::CTRL,
            },
            time: at(10),
        };
        assert_eq!(cycle.handle(quit, &mut fixture.doc), Action::Quit);
        assert_eq!(cycle.pressed.len(), 1, "`a` осталась нажатой");
        cycle.handle(release(KeyCode::Char('a'), at(50)), &mut fixture.doc);
        assert!(cycle.pressed.is_empty());
    }
}

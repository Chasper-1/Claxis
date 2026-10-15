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
            Event::KeyPressed { key, at } => {
                if key.code == KeyCode::Char('q') && key.modifiers == Modifiers::CTRL {
                    return Action::Quit;
                }
                self.pressed.insert(key, at);
                doc.note_press();
            }
            Event::KeyReleased { key, at } => {
                // Отпускание без нажатия — потерянное событие: времени у него
                // нет, и выдумывать ноль секунд вместо неизвестного неправильно.
                if let Some(since) = self.pressed.remove(&key) {
                    doc.note_held(at.saturating_duration_since(since));
                }
            }
            Event::Resized { .. } | Event::FocusGained | Event::FocusLost | Event::Unknown => {}
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

            let event = term.next_event()?;
            if self.handle(event, doc) == Action::Quit {
                return Ok(());
            }
        }
    }
}

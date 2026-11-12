//! Карта строк документа.
//!
//! Карта живёт отдельно от буфера и едет вместе с текстом: буфер знает про
//! байты, карта — про строки. Здесь собрано всё, что карта делает с записью
//! правки: знание о видах записей (`Insert`, `Delete`, `Replace`) больше нигде
//! не размазано.

use crate::api::buffer::Record;
use crate::api::core::LineMap;

/// Карта строк документа: где в тексте переводы строк.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    map: LineMap,
}

impl Lines {
    /// Карта по тексту: документ только что открыт, истории правок ещё не было.
    pub fn build(text: &[u8]) -> Self {
        Self {
            map: LineMap::build(text),
        }
    }

    /// Сама карта: номера строк, позиции, концы строк.
    pub fn map(&self) -> &LineMap {
        &self.map
    }

    /// Вставка текста: переводы строк вставляются вместе с ним.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) {
        self.map.insert(anchor, data);
    }

    /// Вырезан диапазон `len` байт: переводы строк внутри него исчезают.
    pub fn delete(&mut self, anchor: u32, len: u32) {
        self.map.delete(anchor, anchor + len);
    }

    /// Диапазон исчез, на его место пришёл текст.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) {
        if len > 0 {
            self.map.delete(anchor, anchor + len);
        }
        self.map.insert(anchor, data);
    }

    /// Диапазон `[from, to)` вырезан из карты.
    fn cut(&mut self, from: u32, to: u32) {
        self.map.delete(from, to);
    }

    /// Отмена правки: карта возвращается к состоянию до неё.
    ///
    /// `removed` — текст, который запись вырезала из документа. Для вставки его
    /// нет: отменяя вставку, мы убираем вставленное.
    pub fn undo(&mut self, record: &Record, removed: Option<&[u8]>) {
        match record {
            // Отмена вставки: вырезать вставленное.
            Record::Insert { anchor, head, .. } => self.cut(*anchor, *head),
            // Отмена удаления: вернуть вырезанное.
            Record::Delete { anchor, .. } => {
                if let Some(removed) = removed {
                    self.insert(*anchor, removed);
                }
            }
            // Отмена замены: вырезать вставленное и вернуть вырезанное.
            Record::Replace { anchor, head, .. } => {
                if *head > *anchor {
                    self.cut(*anchor, *head);
                }
                if let Some(removed) = removed {
                    self.insert(*anchor, removed);
                }
            }
        }
    }

    /// Повтор правки: карта едет вместе с текстом.
    ///
    /// `inserted` — текст, который запись вставила в документ.
    pub fn redo(&mut self, record: &Record, inserted: Option<&[u8]>) {
        match record {
            Record::Insert { anchor, .. } => {
                if let Some(inserted) = inserted {
                    self.insert(*anchor, inserted);
                }
            }
            Record::Delete { anchor, head, .. } => self.cut(*anchor, *head),
            Record::Replace { anchor, head, .. } => {
                self.cut(*anchor, *head);
                if let Some(inserted) = inserted {
                    self.insert(*anchor, inserted);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lines;
    use crate::api::core::LineMap;

    /// Правка в эталоне: сколько байт затронуто и что было там до неё.
    #[derive(Debug)]
    enum Op {
        Insert { at: usize, len: usize },
        Delete { at: usize, removed: String },
        Replace { at: usize, removed: String },
    }

    #[test]
    fn the_line_map_stays_with_the_document() {
        // Карта строк обязана ехать вместе с текстом на каждой правке, отмене и
        // повторе. Эталон — обычная строка с историей правок: любое расхождение
        // означает, что карта разъехалась.
        //
        // Снимки на диск не пишутся (`keep = 0`): откат упирается в память, и
        // эталон ведёт себя так же. Про откат через кольцо — отдельный тест.
        let mut lines = Lines::build(b"");
        let mut text = String::new();
        let mut history: Vec<Op> = Vec::new();
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };

        for _ in 0..600 {
            // Что именно сделали на этом шаге: без этого расхождение
            // бесполезно.
            let done: String;
            match next(5) {
                0 if !text.is_empty() => {
                    let at = next(text.len());
                    let len = 1 + next((text.len() - at).min(20));
                    let removed = text[at..at + len].to_string();
                    lines.delete(at as u32, len as u32);
                    text.replace_range(at..at + len, "");
                    history.push(Op::Delete { at, removed });
                    done = format!("удалить {at}..{}", at + len);
                }
                1 => {
                    let at = next(text.len() + 1);
                    let typed = if next(2) == 0 { "\n" } else { "ab" };
                    lines.insert(at as u32, typed.as_bytes());
                    text.insert_str(at, typed);
                    history.push(Op::Insert {
                        at,
                        len: typed.len(),
                    });
                    done = format!("вставить {typed:?} на {at}");
                }
                2 => {
                    let at = next(text.len() + 1);
                    let len = next((text.len() - at).min(10));
                    let removed = text[at..at + len].to_string();
                    let typed = "xy\n";
                    lines.replace(at as u32, len as u32, typed.as_bytes());
                    text.replace_range(at..at + len, typed);
                    history.push(Op::Replace { at, removed });
                    done = format!("заменить {at}..{} на {typed:?}", at + len);
                }
                3 => {
                    // Снапшот не меняет текст, и карта строк после него
                    // обязана остаться в силе. Эталон делает то же самое:
                    // история правок остаётся в памяти.
                    history.clear();
                    done = "снапшот".to_string();
                }
                _ => {
                    let Some(op) = history.pop() else { continue };
                    match &op {
                        Op::Insert { at, len } => {
                            lines.undo(
                                &crate::api::buffer::Record::Insert {
                                    anchor: *at as u32,
                                    head: (*at + len) as u32,
                                    text_offset: 0,
                                },
                                None,
                            );
                        }
                        Op::Delete { at, removed } => {
                            lines.undo(
                                &crate::api::buffer::Record::Delete {
                                    anchor: *at as u32,
                                    head: (*at + removed.len()) as u32,
                                    text_offset: 0,
                                },
                                Some(removed.as_bytes()),
                            );
                        }
                        Op::Replace { at, removed } => {
                            lines.undo(
                                &crate::api::buffer::Record::Replace {
                                    anchor: *at as u32,
                                    head: (*at + 3) as u32,
                                    text_offset: 0,
                                    removed_offset: 0,
                                    removed_len: removed.len() as u32,
                                },
                                Some(removed.as_bytes()),
                            );
                        }
                    }
                    done = format!("отменить {op:?}");
                    match op {
                        Op::Insert { at, len } => text.replace_range(at..at + len, ""),
                        Op::Delete { at, removed, .. } => text.insert_str(at, &removed),
                        // Отмена замены убирает вставленное и возвращает
                        // вырезанное.
                        Op::Replace { at, removed, .. } => {
                            text.replace_range(at..at + 3, "");
                            text.insert_str(at, &removed);
                        }
                    }
                }
            }

            let fresh = LineMap::build(text.as_bytes());
            assert_eq!(
                lines.map().breaks(),
                fresh.breaks(),
                "карта строк разошлась с текстом: {done}, текст {:?}",
                text
            );
            assert_eq!(lines.map().line_count(), fresh.line_count());
            assert_eq!(text.as_bytes(), text.as_bytes());
        }
    }

    #[test]
    fn a_line_map_survives_undo_and_replace() {
        // Отмена вставки убирает её переводы строк, отмена удаления и замены
        // возвращает их на место.
        use crate::api::buffer::Record;

        let mut lines = Lines::build(b"");
        lines.insert(0, b"one\ntwo\n");
        assert_eq!(lines.map().breaks(), [3, 7]);
        lines.undo(
            &Record::Insert {
                anchor: 0,
                head: 8,
                text_offset: 0,
            },
            None,
        );
        assert_eq!(lines.map().breaks(), []);

        lines.replace(0, 0, b"a\nb");
        assert_eq!(lines.map().breaks(), [1]);
        lines.undo(
            &Record::Replace {
                anchor: 0,
                head: 3,
                text_offset: 0,
                removed_offset: 0,
                removed_len: 0,
            },
            None,
        );
        assert_eq!(lines.map().breaks(), []);
    }
}

use super::core::Document;
use super::error::{Error, Result};
use super::record::{Record, Version, record_end};
use crate::node::NIL;

/// Запас узлов, который держится про запас до потолка пула.
///
/// Одна правка добавляет узлов не больше, чем высота дерева: балансировка
/// держит её логарифмической, а [`crate::tree`] считает зацикленной глубину
/// сверх 128. Запас в разы больше этой границы, так что проверка перед правкой
/// ловит потолок заранее, а сама правка в него не упирается.
const NODE_HEADROOM: usize = 4096;

impl Document {
    fn check(&self, anchor: u32, len: u32) -> Result<()> {
        // `checked_add` сразу ловит и переполнение на выходе за документ:
        // сложение `as u64` — лишние инструкции в горячем пути.
        if !anchor
            .checked_add(len)
            .is_some_and(|end| end <= self.tree.len)
        {
            return Err(Error::OutOfBounds {
                anchor,
                len,
                doc_len: self.tree.len,
            });
        }
        Ok(())
    }

    /// Есть ли в пуле узлов запас на правку.
    ///
    /// Зовётся до побочных действий: правка, которой не хватило узлов, должна
    /// закончиться ошибкой, ничего не тронув.
    fn ensure_node_room(&self) -> Result<()> {
        if !self.tree.has_room_for_nodes(NODE_HEADROOM) {
            return Err(Error::NodePoolFull {
                nodes: self.tree.slot_len(),
            });
        }
        Ok(())
    }

    // ── правки ───────────────────────────────────────────────────────────

    /// Вставить текст на позицию `anchor`.
    pub fn insert(&mut self, anchor: u32, data: &[u8]) -> Result<()> {
        self.check(anchor, 0)?;
        if data.is_empty() {
            return Ok(());
        }
        // Место проверяется до `discard_redo`: не удавшаяся правка не должна
        // стоить потерянной redo-ветки. `append` потом уже не упадёт.
        self.ensure_buffer_room(data.len())?;
        self.ensure_node_room()?;
        self.discard_redo();
        let text_offset = self.append(data);
        let len = data.len() as u32;
        let record = Record::Insert {
            anchor,
            head: anchor + len,
            text_offset,
        };
        self.apply(record);
        Ok(())
    }

    /// Вырезать диапазон `anchor..anchor+len`.
    pub fn delete(&mut self, anchor: u32, len: u32) -> Result<()> {
        self.check(anchor, len)?;
        if len == 0 {
            return Ok(());
        }
        // Вырезанный кусок равен `len`, и он копируется в хвост: место под
        // копию проверяется до отбрасывания redo-ветки.
        self.ensure_buffer_room(len as usize)?;
        self.ensure_node_room()?;
        self.discard_redo();
        // Вырезанное копируется в хвост буфера: иначе отменять нечем, а
        // второго буфера быть не должно.
        let removed = self.slice(anchor, anchor + len);
        let text_offset = self.append(&removed);
        let record = Record::Delete {
            anchor,
            head: anchor + len,
            text_offset,
        };
        self.apply(record);
        Ok(())
    }

    /// Заменить диапазон: удаление и вставка.
    ///
    /// Одна запись в истории и один проход по дереву: вырезание и вставка
    /// делаются за один раз, а не как две отдельные правки.
    pub fn replace(&mut self, anchor: u32, len: u32, data: &[u8]) -> Result<()> {
        self.check(anchor, len)?;
        if len == 0 && data.is_empty() {
            return Ok(());
        }
        // В хвост уезжают и вырезанное, и вставленное: место под оба куска
        // проверяется до отбрасывания redo-ветки.
        self.ensure_buffer_room(len as usize + data.len())?;
        self.ensure_node_room()?;
        self.discard_redo();

        // Вырезанное копируется в хвост буфера: иначе отменять нечем, а второго
        // буфера быть не должно.
        let removed = self.slice(anchor, anchor + len);
        let removed_offset = self.append(&removed);
        let text_offset = self.append(data);

        let record = Record::Replace {
            anchor,
            head: anchor + data.len() as u32,
            text_offset,
            removed_offset,
            removed_len: len,
        };
        self.apply(record);
        Ok(())
    }

    /// Положить текст в хвост буфера и сказать, с какого места он лежит.
    ///
    /// Буфер только растёт и переезжать не любит: запись правки хранит
    /// смещение, и оно обязано пережить всё, что случится с деревом.
    ///
    /// Место не проверяется: его проверяет вызывающий до побочных действий —
    /// [`Document::ensure_buffer_room`] или [`Document::check_restore`]. Так
    /// на правку приходится одна проверка, а не две.
    #[inline(never)]
    fn append(&mut self, data: &[u8]) -> u32 {
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(data);
        offset
    }

    /// Поместится ли в буфер ещё `extra` байт.
    ///
    /// Зовётся до побочных действий: правка, которой не хватило места, должна
    /// закончиться ошибкой, ничего не тронув. Смещения в буфере 32-битные,
    /// поэтому за четыре гигабайта положить нечего — это ошибка, а не тихая
    /// обрезка до нуля.
    fn ensure_buffer_room(&self, extra: usize) -> Result<()> {
        let end = self.bytes.len() as u64 + extra as u64;
        if end > u32::MAX as u64 {
            return Err(Error::BufferTooLarge { len: end });
        }
        Ok(())
    }

    /// Отменить одну правку обратной операцией.
    ///
    /// Отмена не заканчивается там, где кончилась память: дойдя до конца,
    /// документ подтягивает из приёмника снимок старее и продолжает откат с
    /// него. В памяти лежит всего один снимок, поэтому переход за границу
    /// читает диск — но он происходит один раз на снимок, а не на правку.
    pub fn undo(&mut self) -> Result<Option<u32>> {
        self.ensure_node_room()?;
        // Границ может быть сколько угодно подряд, и каждая со своим пустым
        // хвостом истории. Цикл, а не рекурсия: иначе глубина стека равнялась
        // бы числу снимков.
        while self.current == NIL {
            if !self.step_over_the_boundary_once()? {
                return Ok(None);
            }
        }
        let id = self.current;
        let parent = self.versions[id as usize].parent;
        let record = self.versions[id as usize].record;
        // Всё, что может упасть, проверяется до сдвига вершины: не удавшаяся
        // отмена не должна оставить историю на другой вершине.
        match record {
            // Отмена вставки: вырезать вставленное.
            Record::Insert { anchor, head, .. } => {
                self.current = parent;
                self.tree.cut(anchor, head);
            }
            // Отмена удаления: вернуть вырезанное.
            Record::Delete {
                anchor,
                head,
                text_offset,
            } => {
                let len = head - anchor;
                self.check_restore(text_offset, len)?;
                self.current = parent;
                self.restore(anchor, text_offset, len)?;
            }
            // Отмена замены: вырезать вставленное и вернуть вырезанное.
            Record::Replace {
                anchor,
                head,
                removed_offset,
                removed_len,
                ..
            } => {
                if removed_len > 0 {
                    self.check_restore(removed_offset, removed_len)?;
                }
                self.current = parent;
                self.tree.cut(anchor, head);
                if removed_len > 0 {
                    self.restore(anchor, removed_offset, removed_len)?;
                }
            }
        }
        Ok(Some(id))
    }

    /// Перейти за границу памяти один раз: подтянуть снимок старее.
    ///
    /// `false` — снимков больше нет. Отмену сама не делает: её выполняет цикл
    /// [`Document::undo`], чтобы не уходить в рекурсию на глубину снимков.
    fn step_over_the_boundary_once(&mut self) -> Result<bool> {
        let Some(file) = self.file.clone() else {
            return Ok(false);
        };
        let Some(sink) = self.sink.as_mut() else {
            return Ok(false);
        };
        let Some(history) = sink.older(&file) else {
            return Ok(false);
        };
        let depth = self.depth;
        let snapshot = crate::history::History::decode(&history)
            .and_then(|h| h.restore(depth))
            .map_err(|reason| Error::OlderSnapshotBroken {
                reason: Box::new(reason),
            })?;
        // Счётчики принадлежат документу, а не снимку: снимок — это состояние
        // текста, а не новая сессия. Сбросить их значило бы соврать так же, как
        // врал счётчик самих снимков.
        let snapshots = self.snapshots;
        let compacts = self.compacts;
        let sink = self.sink.take();
        let file = self.file.clone();
        let crossings = self.crossings;
        *self = snapshot;
        // Приёмник и путь переживают смену документа: они принадлежат файлу,
        // а не состоянию. Без них следующая граница была бы последней.
        self.sink = sink;
        self.file = file;
        self.crossings = crossings + 1;
        self.snapshots = snapshots;
        self.compacts = compacts;
        Ok(true)
    }

    /// Проверить, что кусок `from..from+len` есть в буфере и что копия
    /// поместится в него. Ничего не меняет, поэтому зовётся до сдвига истории.
    fn check_restore(&self, from: u32, len: u32) -> Result<()> {
        let end = from
            .checked_add(len)
            .ok_or(Error::HistoryOutOfBounds { offset: from, len })?;
        if end as usize > self.bytes.len() {
            return Err(Error::HistoryOutOfBounds { offset: from, len });
        }
        self.ensure_buffer_room(len as usize)
    }

    /// Положить копию куска в хвост буфера и вставить её на место.
    ///
    /// Копия, а не ссылка на прежнее место: запись, чей текст возвращается,
    /// после отмены уходит в redo-ветку, а redo-ветку следующая правка
    /// отбрасывает вместе с текстом. Лист, смотрящий на этот текст, пережил бы
    /// обрезку буфера и прочитал бы чужое.
    fn restore(&mut self, anchor: u32, from: u32, len: u32) -> Result<()> {
        let end = from
            .checked_add(len)
            .ok_or(Error::HistoryOutOfBounds { offset: from, len })?;
        let restored = self
            .bytes
            .get(from as usize..end as usize)
            .ok_or(Error::HistoryOutOfBounds { offset: from, len })?
            .to_vec();
        let start = self.append(&restored);
        let leaf = self.tree.alloc_leaf(start, len);
        self.tree.insert_leaf(anchor, leaf);
        Ok(())
    }

    /// Повторить отменённую правку.
    pub fn redo(&mut self) -> Result<Option<u32>> {
        let child = self.versions[self.current as usize].child;
        if child == NIL {
            return Ok(None);
        }
        self.ensure_node_room()?;
        let record = self.versions[child as usize].record;
        // Запись проверяется до сдвига вершины: битая запись не должна
        // оставить историю на новой вершине при нетронутом дереве.
        let doc_len = self.tree.len;
        let fits = match record {
            Record::Insert { anchor, head, .. } => anchor <= head && anchor <= doc_len,
            Record::Delete { anchor, head, .. } => anchor <= head && head <= doc_len,
            Record::Replace {
                anchor,
                removed_len,
                ..
            } => anchor as u64 + removed_len as u64 <= doc_len as u64,
        };
        if !fits {
            return Err(Error::OutOfBounds {
                anchor: record.anchor(),
                len: record.len(),
                doc_len,
            });
        }
        self.current = child;
        match record {
            Record::Insert {
                anchor,
                head,
                text_offset,
            } => {
                let leaf = self.tree.alloc_leaf(text_offset, head - anchor);
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
            Record::Replace {
                anchor,
                head,
                text_offset,
                removed_len,
                ..
            } => {
                // Один проход вместо вырезания и вставки: тем же путём идёт
                // разбор истории.
                let leaf = self.tree.alloc_leaf(text_offset, head - anchor);
                self.tree.replace(anchor, removed_len, leaf);
            }
        }
        Ok(Some(child))
    }

    /// Применить запись и сделать её текущей версией.
    ///
    /// Обе разновидности: иначе при разборе истории удаления терялись бы, а
    /// документ собирался бы неверным.
    pub fn apply(&mut self, record: Record) {
        match record {
            Record::Insert {
                anchor,
                head,
                text_offset,
            } => {
                let leaf = self.tree.alloc_leaf(text_offset, head - anchor);
                self.tree.insert_leaf(anchor, leaf);
            }
            Record::Delete { anchor, head, .. } => self.tree.cut(anchor, head),
            Record::Replace {
                anchor,
                head,
                text_offset,
                removed_len,
                ..
            } => {
                // Вырезание и вставка одним проходом: так дерево перестраивается
                // один раз, а не два. Разбор истории идёт тем же путём.
                let leaf = self.tree.alloc_leaf(text_offset, head - anchor);
                self.tree.replace(anchor, removed_len, leaf);
            }
        }
        self.push(record);
    }

    /// Новая вершина истории от текущей.
    fn push(&mut self, record: Record) {
        let live = self.versions[self.current as usize]
            .live
            .max(record_end(&record));
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                self.versions.push(Version {
                    record,
                    parent: NIL,
                    child: NIL,
                    live,
                });
                (self.versions.len() - 1) as u32
            }
        };
        self.versions[id as usize] = Version {
            record,
            parent: self.current,
            child: NIL,
            live,
        };
        let parent = self.current;
        self.versions[parent as usize].child = id;
        self.current = id;
    }

    /// Отбросить redo-ветку: вершины в свободный список, текст в хвост.
    fn discard_redo(&mut self) {
        // Нужен только признак «ветка была»: границу обрезки считает
        // `live_end`, а не самый ранний текст отброшенных записей.
        let mut had_redo = false;
        let mut cursor = self.versions[self.current as usize].child;
        while cursor != NIL {
            let version = self.versions[cursor as usize];
            had_redo = true;
            self.free.push(cursor);
            cursor = version.child;
        }
        self.versions[self.current as usize].child = NIL;
        // Резать можно только то, на что больше никто не смотрит. На хвост
        // буфера ссылаются не только отброшенные записи: живые листья дерева и
        // записи истории на активной ветке. Обрезка оставляет начало буфера,
        // поэтому резать можно только за самым концом живых ссылок — иначе
        // лист останется читать обрезанный участок и потеряет байты.
        if had_redo {
            let end = self.live_end() as usize;
            if end < self.bytes.len() {
                self.bytes.truncate(end);
            }
        }
    }

    /// Самое позднее место в буфере, до которого дотягиваются живые ссылки.
    ///
    /// Три источника. Исходный файл: его лист один и он вечен. Записи живой
    /// ветки: их текст лежит в буфере, хотя в дереве его уже нет, — без него
    /// отмена удаления нечем вернуть. Листья дерева: вставленный текст и те
    /// копии, которые положила отмена.
    ///
    /// Записи ветки берутся готовым числом у вершины, а не обходом: иначе
    /// граница обрезки стоила бы столько же, сколько вся история. Листья тоже
    /// отдают свой конец сразу: его держит сам пул, а не обход дерева.
    fn live_end(&self) -> u32 {
        self.file_len
            .max(self.versions[self.current as usize].live)
            .max(self.tree.leaf_end())
    }

    /// Сколько записей истории занято.
    pub fn history_used(&self) -> u32 {
        self.versions.len() as u32 - self.free.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::support::*;
    use super::*;

    #[test]
    fn insert_splits_and_reads_back() {
        let mut doc = d("abcdefghij");
        doc.insert(5, b"XXXXX").unwrap();
        assert_eq!(doc.read(), b"abcdeXXXXXfghij");
    }

    #[test]
    fn delete_cuts_middle() {
        let mut doc = d("The quick brown fox");
        doc.delete(4, 6).unwrap();
        assert_eq!(doc.read(), b"The brown fox");
    }

    #[test]
    fn replace_is_delete_then_insert() {
        let mut doc = d("The quick brown fox");
        doc.replace(4, 5, b"slow").unwrap();
        assert_eq!(doc.read(), b"The slow brown fox");
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        assert_eq!(doc.read(), b"abcXY");

        doc.undo().unwrap().expect("есть что отменять");
        assert_eq!(doc.read(), b"abc");
        assert!(doc.undo().unwrap().is_none(), "дальше нечего");

        doc.redo().unwrap().expect("есть что повторять");
        assert_eq!(doc.read(), b"abcXY");
        assert!(doc.redo().unwrap().is_none());
    }

    #[test]
    fn undo_of_delete_puts_the_text_back() {
        // Главное: удалённый текст должен вернуться целиком и на своё место.
        let mut doc = d("The quick brown fox");
        doc.delete(4, 5).unwrap();
        assert_eq!(doc.read(), b"The  brown fox");
        doc.undo().unwrap().expect("удаление отменяемо");
        assert_eq!(doc.read(), b"The quick brown fox");
    }

    #[test]
    fn undo_of_a_wide_delete_puts_everything_back() {
        // Удаление через границы листьев — самый частый случай потери текста.
        let mut doc = d("aaa bbb ccc ddd");
        for i in 0..20 {
            doc.insert(i as u32, b"xyz").unwrap();
        }
        let before = doc.read();
        doc.delete(5, 20).unwrap();
        assert_ne!(doc.read(), before);
        doc.undo().unwrap().expect("отменяемо");
        assert_eq!(doc.read(), before, "широкое удаление вернулось не целиком");
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo_branch() {
        let mut doc = d("");
        doc.insert(0, b"A").unwrap();
        doc.insert(1, b"B").unwrap();
        doc.undo().unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.read(), b"");

        doc.insert(0, b"X").unwrap();
        assert_eq!(doc.read(), b"X");
        assert!(doc.redo().unwrap().is_none(), "ветка отброшена");
        assert_eq!(doc.read(), b"X");
    }

    #[test]
    fn repeated_undo_redo_keeps_the_document_intact() {
        let mut doc = d("base");
        for i in 0..10 {
            doc.insert(i as u32, b"ab").unwrap();
        }
        for _ in 0..10 {
            doc.delete(0, 1).unwrap();
        }
        let end = doc.read();
        // Двадцать правок: десять вставок и десять удалений. Отменить надо все.
        for _ in 0..20 {
            doc.undo().unwrap().expect("отменяемо");
        }
        assert_eq!(doc.read(), b"base");
        assert!(doc.undo().unwrap().is_none());
        for _ in 0..20 {
            doc.redo().unwrap().expect("повторяемо");
        }
        assert_eq!(doc.read(), end);
        assert!(doc.redo().unwrap().is_none());
    }

    #[test]
    fn out_of_range_is_an_error_not_a_panic() {
        let mut doc = d("abc");
        assert!(doc.delete(2, 5).is_err());
        assert!(doc.insert(99, b"x").is_err());
        // `anchor + len` за числом документа — тоже ошибка, а не переполнение.
        assert!(doc.delete(u32::MAX, 3).is_err());
        assert!(doc.replace(u32::MAX, 1, b"x").is_err());
        assert_eq!(doc.read(), b"abc");
    }

    #[test]
    fn against_a_reference_string_with_undo_redo_mix() {
        // Эталон — обычная строка. Случайная последовательность правок, отмен и
        // повторов: любое расхождение означает битый документ.
        for seed in 0..8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(7));
            let mut doc = Document::new(b"seed");
            let mut text = String::from("seed");
            let mut steps: Vec<Op> = Vec::new();
            let mut cursor = 0usize;

            for step in 0..1200 {
                let roll = rng.below(12);
                if roll < 4 && !text.is_empty() {
                    let pos = rng.below(text.len());
                    let len = rng.below((text.len() - pos).min(40)) + 1;
                    let removed = text.as_bytes()[pos..pos + len].to_vec();
                    doc.delete(pos as u32, len as u32).unwrap();
                    text.replace_range(pos..pos + len, "");
                    steps.truncate(cursor);
                    steps.push(Op::Delete { pos, removed });
                    cursor = steps.len();
                } else if roll < 7 {
                    let pos = rng.below(text.len() + 1);
                    let typed: String = (0..rng.below(30) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    doc.insert(pos as u32, typed.as_bytes()).unwrap();
                    text.insert_str(pos, &typed);
                    steps.truncate(cursor);
                    steps.push(Op::Insert {
                        pos,
                        text: typed.into_bytes(),
                    });
                    cursor = steps.len();
                } else if roll < 9 {
                    // Замена: самое интересное, потому что это одна запись,
                    // которая несёт и вставленное, и вырезанное.
                    let pos = rng.below(text.len() + 1);
                    let len = rng.below((text.len() - pos).min(40));
                    let removed = text.as_bytes()[pos..pos + len].to_vec();
                    let typed: String = (0..rng.below(20) + 1)
                        .map(|_| (b'a' + rng.below(26) as u8) as char)
                        .collect();
                    doc.replace(pos as u32, len as u32, typed.as_bytes())
                        .unwrap();
                    text.replace_range(pos..pos + len, &typed);
                    steps.truncate(cursor);
                    steps.push(Op::Replace {
                        pos,
                        removed,
                        text: typed.into_bytes(),
                    });
                    cursor = steps.len();
                } else if roll == 9 && cursor > 0 {
                    cursor -= 1;
                    doc.undo()
                        .unwrap()
                        .expect("история должна была быть непустой");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => text.replace_range(*pos..*pos + t.len(), ""),
                        Op::Delete { pos, removed } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(removed))
                        }
                        Op::Replace {
                            pos,
                            removed,
                            text: t,
                        } => {
                            // Отмена замены: убрать вставленное, вернуть
                            // вырезанное.
                            text.replace_range(*pos..*pos + t.len(), "");
                            let back = String::from_utf8_lossy(removed).to_string();
                            text.insert_str(*pos, &back);
                        }
                    }
                } else if cursor < steps.len() {
                    doc.redo()
                        .unwrap()
                        .expect("повтор должен был быть доступен");
                    match &steps[cursor] {
                        Op::Insert { pos, text: t } => {
                            text.insert_str(*pos, &String::from_utf8_lossy(t))
                        }
                        Op::Delete { pos, removed } => {
                            text.replace_range(*pos..*pos + removed.len(), "")
                        }
                        Op::Replace {
                            pos,
                            removed,
                            text: t,
                        } => {
                            let new = String::from_utf8_lossy(t).to_string();
                            text.replace_range(*pos..*pos + removed.len(), &new);
                        }
                    }
                    cursor += 1;
                }

                assert_eq!(
                    doc.read(),
                    text.as_bytes(),
                    "seed {seed} step {step}: текст разошёлся"
                );
                assert_eq!(doc.len() as usize, text.len(), "seed {seed} step {step}");
                assert_eq!(doc.path().len(), cursor, "seed {seed} step {step}");
            }
        }
    }

    #[test]
    fn undo_and_discard_keep_the_buffer_intact() {
        // Последовательность найдена сжатием падения: отмена удаления
        // возвращает текст в конец буфера, отмена замены его вырезает, а
        // следующая правка отбрасывает redo-ветку. Обрезка резала буфер по
        // отброшенным записям и оставляла лист читать участок, которого уже
        // нет.
        let mut doc = Document::new(b"");
        doc.replace(0, 0, b"xy\n").unwrap();
        doc.replace(3, 0, b"xy\n").unwrap();
        doc.delete(1, 4).unwrap();
        doc.undo().unwrap().expect("отменять есть что");
        doc.insert(3, b"ab").unwrap();
        doc.undo().unwrap().expect("отменять есть что");
        doc.undo().unwrap().expect("отменять есть что");
        assert_eq!(doc.read(), b"xy\n");
        doc.insert(2, b"\n").unwrap();
        assert_eq!(doc.read(), b"xy\n\n");
    }

    #[test]
    fn a_big_text_is_inserted_in_one_move() {
        // Кусок больше одного листа: вставка идёт одним куском, а читаться и
        // вырезаться он должен так же, как много мелких.
        let chunk: Vec<u8> = (0..50_000u32).map(|i| b'a' + (i % 26) as u8).collect();
        let mut doc = d("");
        doc.insert(0, &chunk).unwrap();
        assert_eq!(doc.read(), chunk);
        assert_eq!(doc.read_range(10, 20), chunk[10..20].to_vec());

        doc.delete(0, 25_000).unwrap();
        assert_eq!(doc.read(), chunk[25_000..].to_vec());

        // Вставка в середину большого куска режет его надвое.
        doc.insert(10, "ЖЖ".as_bytes()).unwrap();
        let mut expected = chunk[25_000..].to_vec();
        expected.splice(10..10, "ЖЖ".as_bytes().iter().copied());
        assert_eq!(doc.read(), expected);
        assert!(doc.leaves().len() > 1, "большой кусок не разрезался");
    }

    #[test]
    fn unicode_survives_being_cut_anywhere() {
        // Кириллица — два байта на букву. Резать можно где угодно, включая
        // середину буквы: текст от этого не портится, а длина уменьшается ровно
        // на один байт.
        let text = "Привет, мир! Здравствуйте.";
        let mut doc = d(text);
        assert_eq!(doc.read(), text.as_bytes());
        for at in 0..text.len() as u32 {
            doc.delete(at, 1).unwrap();
            assert_eq!(
                doc.read().len(),
                text.len() - 1,
                "вырезан байт {at}: длина должна уменьшиться ровно на один"
            );
            doc.undo().unwrap().expect("удаление отменяемо");
            assert_eq!(doc.read(), text.as_bytes(), "отмена вернула не то");
        }
    }

    #[test]
    fn a_replace_is_one_step_of_history() {
        // Замена — одно действие, значит и один шаг отмены. Раньше это были две
        // записи, и одна команда отменяла половину работы.
        let mut doc = d("The quick brown fox");
        doc.replace(4, 5, b"slow").unwrap();
        assert_eq!(doc.read(), b"The slow brown fox");
        assert_eq!(doc.path().len(), 1, "в истории одна запись, а не две");

        doc.undo().unwrap().expect("замена отменяема");
        assert_eq!(
            doc.read(),
            b"The quick brown fox",
            "вырезанное вернулось целиком"
        );
        doc.redo().unwrap().expect("замена повторяема");
        assert_eq!(doc.read(), b"The slow brown fox");
    }

    #[test]
    fn a_replace_of_a_shorter_text_is_correct() {
        // Длина меняется в обе стороны: короткий кусок вместо длинного и
        // наоборот.
        let mut doc = d("abcdefghij");
        doc.replace(2, 6, b"XY").unwrap();
        assert_eq!(doc.read(), b"abXYij");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"abcdefghij");

        let mut doc = d("abcdefghij");
        doc.replace(4, 1, b"1234567890").unwrap();
        assert_eq!(doc.read(), b"abcd1234567890fghij");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"abcdefghij");
    }

    #[test]
    fn a_replace_across_leaf_borders_is_correct() {
        // Диапазон замены не влезает ни в один лист: это самый частый случай на
        // большом документе.
        let mut doc = d("");
        for i in 0..40u32 {
            doc.insert(i, format!("{i:03}").as_bytes()).unwrap();
        }
        let before = doc.read();
        let inserted = "замена".as_bytes();
        doc.replace(10, 60, inserted).unwrap();
        assert_eq!(doc.read_range(0, 10), before[..10].to_vec());
        assert_eq!(
            doc.read_range(10, 22),
            inserted,
            "вставлено шесть букв, двенадцать байт"
        );
        assert_eq!(doc.read_range(22, 76), before[70..].to_vec());
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), before, "документ вернулся целиком");
    }

    #[test]
    fn a_replace_survives_the_snapshot() {
        // Замена в истории — самая тяжёлая запись: несёт и вставленное, и
        // вырезанное. После снапшота она обязана разобраться целиком.
        let mut doc = d("исходный текст");
        // Вырезаем только слово, пробел остаётся: «новый текст», а не
        // «новыйтекст».
        let cut = "исходный".len() as u32;
        doc.replace(0, cut, "новый".as_bytes()).unwrap();
        let history = history::History::capture(&doc);
        let mut back = history
            .restore(crate::DEFAULT_DEPTH)
            .expect("снапшот разбирается");
        assert_eq!(back.read(), "новый текст".as_bytes());
        back.undo().unwrap().expect("история живая");
        assert_eq!(back.read(), "исходный текст".as_bytes());
    }

    #[test]
    fn a_replace_of_nothing_is_an_insert() {
        // Замена нулевого диапазона — это вставка, но запись остаётся заменой:
        // так видно, что пользователь нажал команду «заменить».
        let mut doc = d("ab");
        doc.replace(1, 0, b"XY").unwrap();
        assert_eq!(doc.read(), b"aXYb");
        doc.undo().unwrap().expect("отменяема");
        assert_eq!(doc.read(), b"ab");
    }

    #[test]
    fn a_replace_out_of_bounds_is_refused() {
        let mut doc = d("abc");
        assert!(doc.replace(2, 5, b"x").is_err());
        assert!(doc.replace(9, 0, b"x").is_err());
        assert_eq!(doc.read(), b"abc", "документ не тронут");
        assert!(doc.path().is_empty(), "в истории пусто");
    }

    #[test]
    fn every_replace_position_and_length_gives_the_right_text() {
        // Перебор всех позиций и длин на маленьких документах: тут находятся
        // случаи, которые случайная последовательность правок не вылавливает,
        // например пустая замена в начале документа.
        for base_len in 0..6usize {
            let base: String = (0..base_len).map(|i| (b'a' + i as u8) as char).collect();
            for pos in 0..=base_len {
                for len in 0..=(base_len - pos) {
                    let mut doc = Document::new(base.as_bytes());
                    doc.replace(pos as u32, len as u32, b"XYZ").unwrap();
                    let mut want = base.clone();
                    want.replace_range(pos..pos + len, "XYZ");
                    assert_eq!(
                        doc.read(),
                        want.as_bytes(),
                        "основа {base:?}, позиция {pos}, длина {len}"
                    );

                    // И отмена обязана вернуть всё как было.
                    doc.undo().unwrap().expect("замена отменяема");
                    assert_eq!(doc.read(), base.as_bytes(), "отмена вернула не то");
                }
            }
        }
    }

    #[test]
    fn the_replace_survives_a_random_mix_with_undo() {
        // Замена с отменами: структура дерева меняется на каждом шаге, и ошибка
        // проявляется не на ровном куске, а на том, что осталось от прежних
        // правок.
        let mut doc = d("");
        let mut text = String::new();
        let mut log: Vec<String> = Vec::new();
        let mut steps: Vec<(u8, usize, usize, String)> = Vec::new();
        let mut rng = Rng(0x2545F4914F6CDD1D);

        for _ in 0..600 {
            match rng.below(4) {
                0 if !text.is_empty() => {
                    let at = rng.below(text.len());
                    let len = 1 + rng.below((text.len() - at).min(20));
                    let removed = text[at..at + len].to_string();
                    doc.delete(at as u32, len as u32).unwrap();
                    text.replace_range(at..at + len, "");
                    steps.push((0, at, len, removed));
                    log.push(format!("delete {at} {len}"));
                }
                1 => {
                    let at = rng.below(text.len() + 1);
                    let typed = if rng.below(2) == 0 { "\n" } else { "ab" };
                    doc.insert(at as u32, typed.as_bytes()).unwrap();
                    text.insert_str(at, typed);
                    steps.push((1, at, typed.len(), String::new()));
                    log.push(format!("insert {at} {typed:?}"));
                }
                2 => {
                    let at = rng.below(text.len() + 1);
                    let len = rng.below((text.len() - at).min(10));
                    let removed = text[at..at + len].to_string();
                    doc.replace(at as u32, len as u32, b"xy\n").unwrap();
                    text.replace_range(at..at + len, "xy\n");
                    steps.push((2, at, 3, removed));
                    log.push(format!("replace {at} {len}"));
                }
                _ => {
                    let Some((kind, at, len, removed)) = steps.pop() else {
                        continue;
                    };
                    doc.undo().unwrap().expect("отменять есть что");
                    match kind {
                        // Отмена вставки: убрать вставленное.
                        1 => text.replace_range(at..at + len, ""),
                        // Отмена удаления: вернуть вырезанное.
                        0 => text.insert_str(at, &removed),
                        // Отмена замены: убрать вставленное и вернуть
                        // вырезанное.
                        _ => {
                            text.replace_range(at..at + len, "");
                            text.insert_str(at, &removed);
                        }
                    }
                    log.push(format!("undo {at} {len}"));
                }
            }

            assert_eq!(
                doc.read(),
                text.as_bytes(),
                "разошлось на {}: {:?}\nвся последовательность: {:?}",
                log.last().unwrap(),
                text,
                log
            );
        }
    }
}

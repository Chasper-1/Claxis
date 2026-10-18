use std::path::PathBuf;

use super::core::Document;
use super::error::{Memory, SnapshotSink};
use super::record::{Record, Version};
use crate::node::{NIL, Node};

impl Document {
    /// Привязать снапшоты к файлу и приёмнику.
    pub fn set_sink(&mut self, file: impl Into<PathBuf>, sink: Box<dyn SnapshotSink>) {
        self.file = Some(file.into());
        self.sink = Some(sink);
    }

    // ── снапшот ──────────────────────────────────────────────────────────

    /// Снять снапшот: текст и вся история уезжают, память встаёт на место снимка.
    ///
    /// В памяти остаётся **один** снимок — тот, что снят последним, — и он же
    /// лежит в кольце на диске. Откат доходит до него по памяти, а уйти дальше
    /// приёмник отдаёт из базы следующий: в базе он уже есть, его читают, а не
    /// забирают.
    pub fn snapshot(&mut self) {
        let document = self.read();
        let history = crate::history::History::capture(self);
        let Some(file) = self.file.clone() else {
            // Снимку некуда уйти, значит он не сделан. Схлопывать память
            // тогда нельзя: история нигде не лежит и пропала бы вместе с
            // возможностью отменить.
            return;
        };
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        self.snapshots += 1;
        sink.accept(&file, &document, history.as_bytes());
        self.compact();
    }

    /// Схлопнуть документ в одну базу и обрезать историю в памяти.
    ///
    /// Память не должна расти вместе с правками, поэтому после снимка
    /// история обрезается, а то, что было до неё, лежит в снимке.
    fn compact(&mut self) {
        let document = self.read();
        self.bytes = document;
        self.file_len = self.bytes.len() as u32;
        self.tree.collapse_to(0, self.file_len);
        self.free.clear();
        self.versions.truncate(1);
        self.versions[0].record = Record::Delete {
            anchor: 0,
            head: 0,
            text_offset: 0,
        };
        self.versions[0].parent = NIL;
        self.versions[0].child = NIL;
        self.current = 0;
        self.compacts += 1;
    }

    /// Сколько раз брался снапшот.
    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots
    }

    /// Сколько раз документ схлопывался после сохранения.
    pub fn compacts_taken(&self) -> u64 {
        self.compacts
    }

    /// Сколько раз откат переходил через границу памяти.
    ///
    /// Число растёт, когда текст сменился целиком: значит, всё построенное по
    /// тексту по частям после этого уже не годится.
    pub fn crossings(&self) -> u64 {
        self.crossings
    }

    // ── диагностика ──────────────────────────────────────────────────────

    /// Статистика памяти.
    pub fn memory(&self) -> Memory {
        Memory {
            nodes: self.tree.node_count(),
            free: self.tree.free_len(),
            capacity: self.tree.capacity(),
            leaves: self.tree.leaves,
            node_size: Node::size(),
            text_len: self.bytes.len(),
            text_capacity: self.bytes.capacity(),
            versions: self.history_used() as usize,
            version_size: std::mem::size_of::<Version>(),
        }
    }

    /// Высота дерева.
    pub fn height(&self) -> u32 {
        self.tree.height()
    }

    /// Листья по порядку документа.
    pub fn leaves(&self) -> Vec<crate::Leaf> {
        self.tree.leaves_in_order()
    }

    /// Диагностика дерева: узлов, высота, худший перекос, расхождение высот.
    pub fn tree_debug_stats(&self) -> crate::tree::TreeStats {
        self.tree.debug_stats()
    }

    /// Дерево обходится целиком и без повторов узлов.
    pub fn tree_is_sound(&self) -> bool {
        self.tree.debug_sound()
    }
}

#[cfg(test)]
mod tests {
    use super::super::core::support::*;
    use super::super::error::{DEFAULT_DEPTH, Result};
    use super::*;

    #[test]
    fn undo_crosses_the_memory_boundary_into_the_ring() {
        // Три снимка, между ними по одной правке. В памяти после каждого
        // снимка пусто, но откат идёт дальше по кольцу — и не должен кончаться
        // на границе.
        let ring = std::rc::Rc::new(std::cell::RefCell::new(Ring::default()));
        let mut doc = Document::new(b"abc");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Share(std::rc::Rc::clone(&ring))),
        );
        for _ in 0..3 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
        }
        assert_eq!(ring.borrow().written.len(), 3, "все три в кольце");
        assert_eq!(doc.path().len(), 0, "в памяти пусто");

        doc.undo().unwrap().expect("откат за границу памяти");
        assert_eq!(doc.read(), b"abcxx");
        doc.undo().unwrap().expect("и следующий снимок");
        assert_eq!(doc.read(), b"abcx");
        doc.undo().unwrap().expect("и третий");
        assert_eq!(doc.read(), b"abc");
        assert!(doc.undo().unwrap().is_none(), "дальше снимков нет");
    }

    #[test]
    fn an_undo_that_crosses_the_boundary_stays_in_memory() {
        // После перехода через границу отказ должен продолжать работать
        // в том же сеансе: границ сколько угодно, и каждая не конец света.
        let ring = std::rc::Rc::new(std::cell::RefCell::new(Ring::default()));
        let mut doc = Document::new(b"");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Share(std::rc::Rc::clone(&ring))),
        );
        for i in 0..4u32 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
            assert_eq!(doc.read(), "x".repeat(i as usize + 1).into_bytes());
        }

        for i in (0..4u32).rev() {
            doc.undo().unwrap().expect("откат идёт");
            assert_eq!(doc.read(), "x".repeat(i as usize).into_bytes());
        }
        assert!(doc.undo().unwrap().is_none());
    }

    #[test]
    fn the_leaf_counter_tells_the_truth() -> Result<()> {
        // Счётчик листьев живёт в дереве, а на него смотрят диагностика и
        // выделение памяти. Разъехаться он может незаметно: листья появляются и
        // исчезают в полудюжине мест, и забыть уменьшить счётчик в одном из
        // них ничего не ломает, кроме самого счётчика. Поэтому он и сверяется
        // здесь с настоящим обходом на смеси правок.
        let mut doc = d("0123456789");
        let mut rng = Rng(7);
        for _ in 0..3000 {
            let len = doc.len();
            let pos = rng.below(len.max(1) as usize) as u32;
            if len > 4 && rng.below(3) == 0 {
                let start = pos.min(len - 2);
                let take = 1 + rng.below(4) as u32;
                doc.delete(start, take.min(len - start))?;
            } else if len >= 2 && rng.below(5) == 0 {
                doc.replace(pos.min(len - 2), 2, b"q")?;
            } else {
                doc.insert(pos, b"xy")?;
            }
            let counted = doc.memory().leaves;
            let real = doc.leaves().len();
            assert_eq!(counted, real, "счётчик листьев разошёлся с деревом");
        }
        Ok(())
    }

    #[test]
    fn typing_into_the_middle_does_not_double_the_leaves() {
        // Каждое нажатие разрезает лист, и без склейки смежных диапазонов
        // дерево удваивалось бы: два листа на нажатие. `concat` склеивает
        // соседей, поэтому на нажатие остаётся один лист.
        let mut doc = Document::from_buffer(vec![b'x'; 64 * 1024], 64 * 1024);
        for _ in 0..1000 {
            doc.insert(32_000, b"y").unwrap();
        }
        // Первое нажатие разрезает лист на три — голова, вставка и хвост, — а
        // каждое следующее добавляет один: новая вставка ложится в документ
        // перед прежними, а в буфер — в самый хвост, после них, поэтому
        // диапазоны не примыкают и склейка не срабатывает. Отсюда около тысячи
        // листьев, а не две тысячи — больше быть не может, но и в один они не
        // сходятся.
        assert_eq!(
            doc.leaves().len(),
            1002,
            "на нажатие должен оставаться один лист"
        );
    }

    #[test]
    fn typing_at_the_end_keeps_one_leaf() {
        // Набор в конец файла — обычный случай. Вставленное примыкает в буфере
        // к предыдущему и смыкается с ним, поэтому дерево остаётся в одном
        // листе. Без склейки на нажатие оставался бы лист, и дерево росло бы
        // вместе с набором.
        let mut doc = Document::new(vec![b'a'; 1000]);
        for _ in 0..500 {
            doc.insert(doc.len(), b"x").unwrap();
        }
        let text = doc.read();
        assert_eq!(text.len(), 1500);
        assert!(text.ends_with(&vec![b'x'; 500]));
        assert_eq!(
            doc.leaves().len(),
            1,
            "набор в конец должен держать один лист"
        );
    }

    #[test]
    fn undoing_everything_puts_the_leaves_back_together() {
        // Отмена вставки вырезает вставленное, и оставшиеся куски листа
        // снова смыкаются в буфере. Дерево после полного отката обязано
        // вернуться к одному листу: иначе отмена не «схлопывает» документ, а
        // оставляет после себя мусор, который растёт с каждой серией правок.
        let mut doc = d("0123456789");
        for _ in 0..200 {
            doc.insert(0, b"xy").unwrap();
        }
        assert!(doc.leaves().len() > 1, "правки должны были порезать лист");
        while doc.undo().unwrap().is_some() {}
        assert_eq!(doc.read(), b"0123456789");
        assert_eq!(doc.leaves().len(), 1, "после отката всё должно слипться");
    }

    #[test]
    fn merging_leaves_keeps_the_text_and_the_history() {
        // Склейка не должна ломать ни текст, ни отмену: запись правки живёт в
        // байтах и границ листьев не знает, но проверить это надо на живом
        // документе, где склейка действительно происходит.
        let mut doc = d("0123456789");
        doc.insert(5, b"ab").unwrap();
        doc.delete(0, 3).unwrap();
        doc.replace(1, 2, b"ZZ").unwrap();
        let end = doc.read();
        let leaves_after_edits = doc.leaves().len();

        for _ in 0..3 {
            doc.undo().unwrap().expect("отменяемо");
        }
        assert_eq!(doc.read(), b"0123456789");
        for _ in 0..3 {
            doc.redo().unwrap().expect("повторяемо");
        }
        assert_eq!(doc.read(), end);
        assert_eq!(
            doc.leaves().len(),
            leaves_after_edits,
            "повтор сломал листья"
        );

        // Полный откат после серии правок обязан вернуть исходный текст байт
        // в байт: буфер мог порезаться при отброшенной ветке.
        while doc.undo().unwrap().is_some() {}
        assert_eq!(doc.read(), b"0123456789");
    }

    #[test]
    fn a_snapshot_carries_text_and_history() {
        // Снапшот уносит состояние целиком: и текст, и историю. Без истории
        // документ верный, а отменять нечего.
        let (text, history) = {
            let mut doc = d("abc");
            doc.set_sink(
                std::path::PathBuf::from("/tmp/d.txt"),
                Box::new(Attached::default()),
            );
            doc.insert(3, b"XY").unwrap();
            doc.delete(0, 1).unwrap();
            // История снимается до снапшота: снимок её уводит, это его смысл.
            let history = history::History::capture(&doc);
            let text = doc.read();
            doc.snapshot();
            assert_eq!(doc.read(), text, "снапшот текст не меняет");
            (text, history)
        };
        assert_eq!(text, b"bcXY");

        let mut back = history.restore().expect("снапшот разбирается");
        assert_eq!(back.read(), text);
        back.undo().unwrap().expect("история восстановлена");
        assert_eq!(back.read(), b"abcXY");
    }

    #[test]
    fn a_snapshot_keeps_only_the_newest_edit_in_memory() {
        // Снапшот схлопывает память: всё, что было до него, уезжает в кольцо,
        // а в памяти остаётся ровно один снимок. Откат доходит до него без
        // диска, дальше — приёмник отдаёт следующий из базы.
        let mut doc = Document::with_depth(b"abc", 4).unwrap();
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Attached::default()),
        );
        for i in 0..100 {
            doc.insert(i as u32, b"x").unwrap();
        }
        assert_eq!(doc.path().len(), 100, "история росла вместе с правками");

        doc.snapshot();
        assert_eq!(doc.snapshots_taken(), 1, "по прямой просьбе снимается");
        assert_eq!(doc.path().len(), 0, "в памяти один снимок и ничего");
        assert_eq!(doc.read().len(), 103, "текст на месте");
    }

    #[test]
    fn compacting_cuts_the_history_on_purpose() {
        // Схлопывание — отдельная операция после сохранения файла. Вот тут
        // история обрезается, и это осознанно.
        let mut doc = Document::new(b"abc");
        doc.insert(3, b"XY").unwrap();
        assert_eq!(doc.path().len(), 1);

        doc.compact();
        assert_eq!(doc.compacts_taken(), 1);
        assert_eq!(doc.path().len(), 0, "после схлопывания история пуста");
        assert_eq!(doc.read(), b"abcXY", "текст при этом цел");
        assert!(doc.undo().unwrap().is_none(), "отменять больше нечего");

        // И документ продолжает работать.
        doc.insert(5, b"Z").unwrap();
        assert_eq!(doc.read(), b"abcXYZ");
    }

    #[test]
    fn a_snapshot_leaves_undo_working_after_it() {
        // Главное про память: отмена после снимка живая. Дальше по границу
        // уходит приёмник, а не обрыв цепочки.
        let mut doc = Document::new(b"abc");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Attached::default()),
        );
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();

        doc.insert(5, b"Z").unwrap();
        assert_eq!(doc.read(), b"abcXYZ");
        doc.undo().unwrap().expect("правка после снимка отменяется");
        assert_eq!(doc.read(), b"abcXY");
    }

    #[test]
    fn the_newest_snapshot_replaces_the_previous_one_in_memory() {
        // В памяти снимок один: новый вытесняет старый, но старый остаётся в
        // кольце. Откат за границу идёт по ним в порядке убывания.
        let mut doc = Document::new(b"abc");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(Attached::default()),
        );
        for _ in 0..5 {
            doc.insert(doc.len(), b"x").unwrap();
            doc.snapshot();
        }
        assert_eq!(doc.snapshots_taken(), 5);
        assert_eq!(doc.path().len(), 0, "в памяти один снимок, не пять");
        assert_eq!(doc.read(), b"abcxxxxx");
    }

    #[test]
    fn the_tree_stays_balanced_under_replaces_and_undos() {
        // Тот же баланс, но под заменами и отменами: это самый дорогой путь
        // дерева, и рассинхрон высот там виден не сразу, а потом всплывает
        // медленным спуском по документу.
        // Текст латиницей: позиции в эталоне байтовые, и резать строку посреди
        // многобайтового символа нельзя — тест проверяет дерево, а не UTF-8.
        let mut doc = Document::new("start\n".as_bytes());
        let mut rng = Rng(0x5DEECE66D);
        let mut reference = String::from("start\n");
        // Последняя правка была заменой: что вставили и что там было.
        // Отменять умеем только её, поэтому и отменяем только её.
        let mut last_replace: Option<(usize, String)> = None;
        const INSERTED: &str = "replaced\n";

        for _ in 0..1500 {
            match rng.below(5) {
                0 if last_replace.is_some() => {
                    let (pos, before) = last_replace.take().expect("замена была");
                    let after_redo = reference.clone();
                    doc.undo().unwrap().expect("отменять есть что");
                    // Отмена замены: убрать вставленное и вернуть вырезанное.
                    reference.replace_range(pos..pos + INSERTED.len(), &before);
                    assert_eq!(
                        doc.read(),
                        reference.as_bytes(),
                        "отмена замены на {pos} вернула не то: было {:?}, стало {:?}",
                        after_redo,
                        before
                    );
                    doc.redo().unwrap().expect("повторять есть что");
                    // Повтор вернул документ к тому, что было: эталон тоже.
                    reference = after_redo;
                    assert_eq!(doc.read(), reference.as_bytes());
                }
                1 => {
                    let pos = rng.below(reference.len() + 1);
                    let len = rng.below((reference.len() - pos).min(20));
                    let before = reference[pos..pos + len].to_string();
                    doc.replace(pos as u32, len as u32, INSERTED.as_bytes())
                        .unwrap();
                    reference.replace_range(pos..pos + len, INSERTED);
                    last_replace = Some((pos, before));
                }
                2 if !reference.is_empty() => {
                    let pos = rng.below(reference.len());
                    let len = 1 + rng.below((reference.len() - pos).min(10));
                    last_replace = None;
                    doc.delete(pos as u32, len as u32).unwrap();
                    reference.replace_range(pos..pos + len, "");
                }
                _ => {
                    let pos = rng.below(reference.len() + 1);
                    last_replace = None;
                    doc.insert(pos as u32, "inserted\n".as_bytes()).unwrap();
                    reference.insert_str(pos, "inserted\n");
                }
            }
            assert_eq!(doc.read(), reference.as_bytes());
        }

        let stats = doc.tree_debug_stats();
        assert_eq!(
            stats.height_mismatch, 0,
            "высота в узле разошлась с настоящей"
        );
        assert!(
            stats.worst_imbalance <= 1,
            "перекос {} после замен и отмен",
            stats.worst_imbalance
        );
        assert_eq!(stats.height, doc.height(), "высота в корне не совпадает");
    }

    #[test]
    fn tree_stays_balanced_under_churn() -> Result<()> {
        // Баланс держится при любом порядке правок: иначе спуск перестаёт быть
        // логарифмическим.
        let mut doc = Document::new(b"a");
        let mut rng = Rng(99);
        for _ in 0..2000 {
            let len = doc.len();
            let pos = rng.below(len as usize + 1) as u32;
            if rng.below(2) == 0 && len >= 1 {
                let start = pos.min(len - 1);
                let take = 1 + rng.below(len as usize) as u32;
                doc.delete(start, take.min(len - start))?;
            } else {
                doc.insert(pos, b"xy")?;
            }
        }
        let stats = doc.tree_debug_stats();
        let leaves = doc.leaves().len();
        if leaves > 0 {
            // Сплошное дерево из n листьев всегда ровно 2n − 1 узлов.
            assert_eq!(stats.nodes, leaves * 2 - 1, "дерево не сплошное");
        }
        assert_eq!(
            stats.height_mismatch, 0,
            "высота в узле разошлась с настоящей: баланс считается не по тому"
        );
        assert!(
            stats.worst_imbalance <= 1,
            "перекос {} при {} узлах",
            stats.worst_imbalance,
            stats.nodes
        );
        assert_eq!(stats.height, doc.height(), "высота в корне не совпадает");
        // Высота AVL: не больше примерно 1.44 от log2(листьев) плюс запас.
        let height = stats.height;
        let bound = (leaves.max(2) as f64).log2() * 1.5 + 2.0;
        assert!(
            height as f64 <= bound,
            "высота {height} при {leaves} листьях превышает {bound:.1}"
        );
        Ok(())
    }

    #[test]
    fn memory_does_not_run_away() {
        let mut doc = Document::new(b"");
        for i in 0..8000u32 {
            doc.insert(i % (doc.len() + 1), b"xyz").unwrap();
        }
        let m = doc.memory();
        eprintln!(
            "ROPE 8000 правок | листьев {:>5} | узлов {:>5}/{} | вершин {:>5} | текст {:>6}/{} | ПУЛ {:>7} | вершины {:>7} | всего {:>7}",
            m.leaves,
            m.nodes,
            m.capacity,
            m.versions,
            m.text_len,
            m.text_capacity,
            m.tree_bytes(),
            m.versions * m.version_size,
            m.tree_bytes() + m.text_capacity + m.versions * m.version_size,
        );
        assert_eq!(doc.read().len(), 24_000);
        assert_eq!(Document::new(b"").history_depth(), DEFAULT_DEPTH);
    }

    #[test]
    fn a_snapshot_survives_a_random_history() -> Result<()> {
        // Снапшот снимается в произвольный момент под нагрузкой: восстановленный
        // документ должен совпасть и текстом, и историей.
        for seed in 0..4u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
            let mut doc = Document::new("начало".as_bytes());
            for _ in 0..200 {
                let len = doc.len();
                let pos = rng.below(len as usize + 1) as u32;
                if rng.below(2) == 0 && len >= 1 {
                    let start = pos.min(len - 1);
                    let take = 1 + rng.below(20) as u32;
                    doc.delete(start, take.min(len - start))?;
                } else {
                    doc.insert(pos, b"qq")?;
                }
                if rng.below(7) == 0 {
                    assert!(doc.undo()?.is_some(), "seed {seed}: undo без истории");
                }
            }
            let text = doc.read();
            let captured = history::History::capture(&doc);
            let mut back = captured.restore()?;
            assert_eq!(back.read(), text, "seed {seed}: текст разошёлся");
            assert_eq!(
                back.path().len(),
                doc.path().len(),
                "seed {seed}: история разошлась"
            );

            // Отмена в восстановленном документе идёт по той же ветке.
            for _ in 0..doc.path().len() {
                assert!(
                    back.undo()?.is_some(),
                    "seed {seed}: история кончилась раньше времени"
                );
            }
            assert_eq!(
                back.read(),
                "начало".as_bytes(),
                "seed {seed}: откат до начала не сошёлся"
            );
        }
        Ok(())
    }
}

//! Проверки внутренних инвариантов буфера и недвижимые тесты.
//!
//! Обычные тесты проверяют текст. Здесь проверяется то, что иначе может
//! тихо разъехаться: целостность пула узлов, баланс дерева, сходимость
//! частичного чтения с полным, поведение арены на границах блоков и
//! соответствие эталону на длинных случайных последовательностях.

use crate::arena::{ArenaSize, Record, RecordId};
use crate::buffer::Buffer;

/// Короткий конструктор буфера из строки.
fn b(text: &str) -> Buffer {
    Buffer::new(text.as_bytes())
}

// ── инварианты дерева ────────────────────────────────────────────────

/// Проверить, что дерево цело: листья, баланс и учёт узлов сходятся.
fn check_tree(buf: &Buffer, label: &str) {
    let leaves = buf.segments().len();
    let height = buf.tree_height();

    // Полное двоичное дерево: внутренних узлов ровно на один меньше, чем
    // листьев. Любая утечка или двойное освобождение узла это ломает.
    let mem = buf.tree_memory();
    let used = mem.nodes;
    let expected = if leaves == 0 { 0 } else { 2 * leaves - 1 };
    assert_eq!(
        used, expected,
        "{label}: занято {used} узлов, листьев {leaves}, ожидалось {expected}"
    );

    // Высота AVL-дерева ограничена: минимум листьев для высоты h — это
    // числа Фибоначчи, F(h+1). Логарифмическая оценка 1.44*log2(n) верна
    // только для больших n и на малых ошибается, поэтому считаем точно.
    if leaves > 0 {
        let max_height = max_avl_height(leaves);
        assert!(
            height <= max_height,
            "{label}: высота {height} при {leaves} листьях превышает {max_height}"
        );
        assert!(height >= 1);
    } else {
        assert_eq!(height, 0, "{label}: пустое дерево должно иметь высоту 0");
    }

    // Лист не может быть пустым: пустой сегмент означал бы, что о нём
    // забыли и он навсегда остался бы в дереве.
    for (i, s) in buf.segments().iter().enumerate() {
        assert!(s.len > 0, "{label}: пустой сегмент #{i}");
    }
}

/// Максимальная высота AVL-дерева с `leaves` листьями.
///
/// Минимум листьев для высоты h равен F(h+1): F(1)=1, F(2)=1, F(3)=2… —
/// это в точности рекурсия F(h) = F(h-1) + F(h-2) из определения баланса.
fn max_avl_height(leaves: usize) -> usize {
    // fib[k] — число листьев, достаточное для высоты k.
    let (mut prev, mut cur) = (0usize, 1usize); // F(0), F(1)
    let mut height = 0usize;
    // F(height+1) <= leaves
    while cur <= leaves {
        height += 1;
        let next = prev + cur;
        prev = cur;
        cur = next;
    }
    height
}

#[test]
fn tree_accounting_holds_under_random_edits() {
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                return 0;
            }
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    for seed in 0..8u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));
        let mut buf = Buffer::new(b"start");
        let mut text = String::from("start");
        for step in 0..400 {
            if text.is_empty() || rng.below(3) != 0 {
                let pos = rng.below(text.len() + 1);
                let piece: String = (0..rng.below(4) + 1)
                    .map(|_| (b'a' + rng.below(26) as u8) as char)
                    .collect();
                buf.insert(pos as u32, piece.as_bytes()).unwrap();
                text.insert_str(pos, &piece);
            } else {
                let pos = rng.below(text.len());
                let len = rng.below(text.len() - pos);
                buf.delete(pos as u32, len as u32).unwrap();
                text.drain(pos..pos + len);
            }
            assert_eq!(buf.read(), text.as_bytes(), "seed {seed} step {step}");
            assert_eq!(buf.len() as usize, text.len());
            check_tree(&buf, &format!("seed {seed} step {step}"));
        }
    }
}

#[test]
fn tree_accounting_holds_after_undo_and_redo_walk() {
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                return 0;
            }
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    let mut rng = Rng(0xC0FFEE);
    let mut buf = Buffer::new(b"");
    let mut history = 0usize;

    for _ in 0..600 {
        if rng.below(4) != 0 {
            let pos = rng.below(buf.len() as usize + 1) as u32;
            let piece: Vec<u8> = (0..rng.below(5) + 1)
                .map(|_| b'a' + rng.below(26) as u8)
                .collect();
            buf.insert(pos, &piece).unwrap();
            history += 1;
        } else if !buf.is_empty() {
            let pos = rng.below(buf.len() as usize) as u32;
            let len = rng.below((buf.len() - pos) as usize) as u32 + 1;
            buf.delete(pos, len).unwrap();
            history += 1;
        }
        check_tree(&buf, "после правки");
    }

    // Отматываем всё до нуля: дерево обязано схлопнуться без остатка.
    let mut undone = 0;
    while buf.undo().is_some() {
        undone += 1;
        check_tree(&buf, "после undo");
        assert!(undone <= history);
    }
    assert_eq!(undone, history, "отменилось не всё");
    assert_eq!(buf.len(), 0, "документ обязан вернуться к исходному");

    // И возвращаем всё обратно.
    let mut redone = 0;
    while buf.redo().is_some() {
        redone += 1;
        check_tree(&buf, "после redo");
    }
    assert_eq!(redone, history);
}

// ── чтение ───────────────────────────────────────────────────────────

#[test]
fn every_slice_matches_full_read() {
    // Полное пересечение срезов: для каждой пары границ частичное чтение
    // обязано совпасть с подстрокой полного. Ловит ошибки баз в рекурсии,
    // которые выборочные проверки пропускают.
    let mut buf = Buffer::new(b"");
    for i in 0..60u8 {
        buf.insert(buf.len(), &[b'a' + i]).unwrap();
    }
    buf.insert(10, b"MIDDLE").unwrap();
    buf.delete(20, 5).unwrap();

    let full = buf.read();
    let n = full.len();
    for start in 0..=n {
        for end in start..=n {
            assert_eq!(
                buf.read_range(start as u32, end as u32),
                full[start..end],
                "срез {start}..{end}"
            );
        }
    }
}

#[test]
fn read_range_clamps_out_of_bounds() {
    let buf = b("0123456789");
    assert_eq!(buf.read_range(5, 5), b"");
    assert_eq!(buf.read_range(0, 1000), b"0123456789");
    assert_eq!(buf.read_range(1000, 2000), b"");
    assert!(buf.read_range(500, 100).is_empty());
}

// ── границы документа ────────────────────────────────────────────────

#[test]
fn empty_document_behaves() {
    let mut buf = Buffer::new(b"");
    assert!(buf.is_empty());
    assert_eq!(buf.read(), b"");
    assert_eq!(buf.tree_height(), 0);
    check_tree(&buf, "пустой");

    assert!(buf.delete(0, 1).is_err());
    assert!(buf.insert(1, b"x").is_err());
    buf.insert(0, b"x").unwrap();
    assert_eq!(buf.read(), b"x");
    check_tree(&buf, "после вставки в пустой");

    buf.delete(0, 1).unwrap();
    assert!(buf.is_empty());
    check_tree(&buf, "после удаления всего");
}

#[test]
fn insert_and_delete_at_every_boundary() {
    let mut buf = Buffer::new(b"");
    buf.insert(0, b"0123456789").unwrap();
    let full = buf.read();
    let n = full.len() as u32;

    // Вставка в каждую возможную точку.
    for pos in 0..=n {
        let mut b2 = Buffer::new(&full[..]);
        b2.insert(pos, b"#").unwrap();
        let mut want = full.clone();
        want.insert(pos as usize, b'#');
        assert_eq!(b2.read(), want, "вставка в {pos}");
        check_tree(&b2, &format!("вставка в {pos}"));
    }

    // Удаление каждого возможного диапазона.
    for pos in 0..n {
        for len in 1..=(n - pos) {
            let mut b2 = Buffer::new(&full[..]);
            b2.delete(pos, len).unwrap();
            let mut want = full.clone();
            want.drain(pos as usize..(pos + len) as usize);
            assert_eq!(b2.read(), want, "удаление {pos}..{}", pos + len);
            check_tree(&b2, &format!("удаление {pos}..{}", pos + len));
        }
    }
}

// ── арена ────────────────────────────────────────────────────────────

#[test]
fn arena_size_ladder_values_and_records() {
    let ladder = [
        (ArenaSize::Kb32, 2_048usize),
        (ArenaSize::Kb64, 4_096),
        (ArenaSize::Kb96, 6_144),
        (ArenaSize::Kb128, 8_192),
        (ArenaSize::Kb192, 12_288),
        (ArenaSize::Kb256, 16_384),
        (ArenaSize::Kb320, 20_480),
        (ArenaSize::Kb384, 24_576),
        (ArenaSize::Kb448, 28_672),
        (ArenaSize::Kb512, 32_768),
        (ArenaSize::Kb640, 40_960),
        (ArenaSize::Kb768, 49_152),
        (ArenaSize::Kb896, 57_344),
        (ArenaSize::Kb1024, 65_536),
    ];
    for (size, records) in ladder {
        assert_eq!(size.records(), records, "{size:?}: неверное число записей");
        assert_eq!(size.records() * size_of_record(), size.bytes());
    }
    // Лесенка строго возрастает, максимум — 1 МБ.
    let mut prev = 0;
    for (size, _) in ladder {
        assert!(
            size.bytes() > prev,
            "{size:?}: должен быть больше предыдущего"
        );
        prev = size.bytes();
    }
    assert_eq!(prev, 1024 * 1024);
}

fn size_of_record() -> usize {
    core::mem::size_of::<Record>()
}

#[test]
fn record_ids_are_stable_and_unique_across_blocks() {
    // Много записей в малой арене: блоков несколько, идентификаторы не
    // должны ни повторяться, ни уезжать после добавления новых блоков.
    let mut buf = Buffer::with_arena_size(b"", ArenaSize::Kb32);
    let mut ids: Vec<RecordId> = Vec::new();
    for i in 0..3_000u32 {
        buf.insert(i, b"x").unwrap();
        ids.push(*buf.undo_stack().last().unwrap());
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "идентификаторы повторились");

    let blocks = buf.tree_memory().arena_blocks;
    assert!(
        blocks > 1,
        "арена должна была разбиться на блоки, блоков {blocks}"
    );

    // Все записи читаются и дают ожидаемые диапазоны.
    for &id in &ids {
        let rec = buf.record(id).expect("запись пропала");
        let _ = rec.anchor();
        let _ = rec.head();
        assert_eq!(rec.len(), rec.anchor().abs_diff(rec.head()));
    }
    check_tree(&buf, "после 3000 вставок через несколько блоков");
}

#[test]
fn record_len_and_text_accessors() {
    let mut buf = b("");
    buf.insert(0, b"abc").unwrap();
    let id = *buf.undo_stack().last().unwrap();
    let rec = buf.record(id).unwrap();
    assert_eq!(rec.anchor(), 0);
    assert_eq!(rec.head(), 3);
    assert_eq!(rec.len(), 3);
    assert!(!rec.is_empty());
    assert!(rec.text().is_some(), "вставка несёт ссылку на текст");

    buf.delete(0, 1).unwrap();
    let id = *buf.undo_stack().last().unwrap();
    let rec = buf.record(id).unwrap();
    assert_eq!(rec.len(), 1);
    assert!(
        rec.text().is_none(),
        "удаление текста не добавляет, значит ссылки быть не должно"
    );
}

// ── снапшот и память ─────────────────────────────────────────────────

#[test]
fn memory_stays_bounded_through_long_session() {
    // Долгая сессия: листья не должны уходить за порог ни разу.
    let limit = Buffer::MAX_LEAVES as usize;
    let mut buf = Buffer::with_arena_size(b"", ArenaSize::Kb128);
    let mut pos = 0u32;
    for i in 0..60_000 {
        pos = (pos + 13) % buf.len().max(1);
        buf.insert(pos, b"payload").unwrap();
        if i % 3 == 0 && buf.len() > 20 {
            buf.delete(pos, 3).unwrap();
        }
        let leaves = buf.segments().len();
        assert!(
            leaves <= limit,
            "на шаге {i} листьев {leaves} при пороге {limit}"
        );
    }
    assert!(buf.snapshots_taken() > 0, "снапшот обязан был сработать");
    check_tree(&buf, "после длинной сессии");
}

#[test]
fn node_pool_is_fixed_size_and_never_grows() {
    // Пул выделяется один раз под потолок и больше не растёт: ни при
    // постройке дерева, ни при волнах вставок и удалений.
    let buf = Buffer::new(b"tiny");
    let mem = buf.tree_memory();
    assert_eq!(
        mem.pool_capacity,
        2 * Buffer::MAX_LEAVES as usize + 8,
        "ёмкость пула выведена из потолка листьев"
    );
    // 32767 занятых максимум — это и есть 512 КБ при узле 16 байт.
    assert_eq!(mem.tree_bytes(), (2 * 16 * 1024 + 8) * 16);
}

#[test]
fn node_pool_reused_across_churn() {
    // Волны вставок и удалений не должны раздувать пул: узлы возвращаются
    // в свободные и берутся снова.
    let mut buf = Buffer::with_arena_size(b"", ArenaSize::Kb128);
    let pool = buf.tree_memory().pool_capacity;
    for _ in 0..5_000 {
        buf.insert(buf.len(), b"chunk").unwrap();
    }
    assert_eq!(buf.len(), 25_000);
    assert_eq!(buf.tree_memory().pool_capacity, pool, "пул изменился");
    let peak_nodes = buf.tree_memory().nodes;

    while !buf.is_empty() {
        buf.delete(0, 1).unwrap();
    }
    check_tree(&buf, "после полного удаления");

    for _ in 0..5_000 {
        buf.insert(buf.len(), b"chunk").unwrap();
    }
    let now_nodes = buf.tree_memory().nodes;
    assert!(
        now_nodes <= peak_nodes + 16,
        "пул вырос: {now_nodes} против {peak_nodes}"
    );
    assert_eq!(buf.read().len(), 25_000);
}

#[test]
fn snapshot_clears_history_and_keeps_text() {
    let mut buf = b("hello");
    buf.insert(5, b" world").unwrap();
    buf.insert(11, b"!").unwrap();
    assert_eq!(buf.read(), b"hello world!");

    let before = buf.snapshots_taken();
    buf.snapshot();
    assert_eq!(buf.snapshots_taken(), before + 1);
    assert_eq!(buf.read(), b"hello world!");
    assert_eq!(buf.original(), b"hello world!");
    assert!(buf.undo_stack().is_empty());
    assert!(buf.redo_stack().is_empty());
    assert!(buf.undo().is_none(), "истории после снапшота нет");
    check_tree(&buf, "после снапшота");

    // Дальше буфер работает как обычно.
    buf.insert(12, b"?").unwrap();
    assert_eq!(buf.read(), b"hello world!?");
    buf.undo().unwrap();
    assert_eq!(buf.read(), b"hello world!");
}

// ── соответствие эталону на длинных прогонах ─────────────────────────

#[test]
fn matches_reference_with_undo_redo_mix() {
    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                return 0;
            }
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    /// Что случилось с текстом — чтобы уметь откатить и вернуть.
    enum Op {
        Insert { pos: usize, text: Vec<u8> },
        Delete { pos: usize, removed: Vec<u8> },
    }

    for seed in 0..12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(7));
        let mut buf = Buffer::new(b"seed");
        let mut text = String::from("seed");
        let mut history: Vec<Op> = Vec::new();
        // Курсор в истории: всё до него применено, всё после — redo.
        let mut cursor = 0usize;

        for step in 0..1500 {
            let roll = rng.below(10);
            if roll < 5 && !text.is_empty() {
                // Новая правка: redo-ветка отменяется, новая операция в конец.
                // Позиция строго меньше длины: удалять нечего на самом краю.
                let pos = rng.below(text.len());
                let len = rng.below((text.len() - pos).min(40)) + 1;
                let removed = text.as_bytes()[pos..pos + len].to_vec();
                buf.delete(pos as u32, len as u32).unwrap();
                text.replace_range(pos..pos + len, "");
                history.truncate(cursor);
                history.push(Op::Delete { pos, removed });
                cursor = history.len();
            } else if roll < 8 {
                // Вставка.
                let pos = rng.below(text.len() + 1);
                let piece: String = (0..rng.below(30) + 1)
                    .map(|_| (b'a' + rng.below(26) as u8) as char)
                    .collect();
                buf.insert(pos as u32, piece.as_bytes()).unwrap();
                text.insert_str(pos, &piece);
                history.truncate(cursor);
                history.push(Op::Insert {
                    pos,
                    text: piece.into_bytes(),
                });
                cursor = history.len();
            } else if roll == 8 && cursor > 0 {
                // Отмена.
                cursor -= 1;
                buf.undo().expect("история должна была быть непустой");
                match &history[cursor] {
                    Op::Insert { pos, text: t } => text.replace_range(*pos..*pos + t.len(), ""),
                    Op::Delete { pos, removed } => {
                        text.insert_str(*pos, &String::from_utf8_lossy(removed))
                    }
                }
            } else if cursor < history.len() {
                // Повтор.
                buf.redo().expect("redo должен был быть доступен");
                match &history[cursor] {
                    Op::Insert { pos, text: t } => {
                        text.insert_str(*pos, &String::from_utf8_lossy(t))
                    }
                    Op::Delete { pos, removed } => {
                        text.replace_range(*pos..*pos + removed.len(), "")
                    }
                }
                cursor += 1;
            }

            assert_eq!(
                buf.read(),
                text.as_bytes(),
                "seed {seed} step {step}: текст разошёлся"
            );
            assert_eq!(buf.len() as usize, text.len(), "seed {seed} step {step}");
            assert_eq!(buf.undo_stack().len(), cursor, "seed {seed} step {step}");
        }

        // Финальная сверка: отменяем всё до старта и возвращаем обратно.
        while buf.undo().is_some() {
            cursor -= 1;
        }
        assert_eq!(cursor, 0);
        assert_eq!(buf.read(), b"seed", "seed {seed}: не вернулся к началу");

        while buf.redo().is_some() {
            cursor += 1;
        }
        assert_eq!(cursor, history.len());
        assert_eq!(
            buf.read(),
            text.as_bytes(),
            "seed {seed}: не вернулся в конец"
        );
    }
}

#[test]
fn replace_matches_delete_then_insert() {
    let mut buf = b("The quick brown fox");
    buf.replace(4, 5, b"slow").unwrap();
    assert_eq!(buf.read(), b"The slow brown fox");
    // Первая отмена убирает вставку — остаётся документ без "quick".
    buf.undo().unwrap();
    assert_eq!(buf.read(), b"The  brown fox");
    // Вторая отмена возвращает удалённое.
    buf.undo().unwrap();
    assert_eq!(buf.read(), b"The quick brown fox");
    check_tree(&buf, "после replace и отмен");
}

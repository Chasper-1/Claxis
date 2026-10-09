use crate::buffer::Buffer;

fn b(text: &str) -> Buffer {
    Buffer::new(text.as_bytes())
}

#[test]
fn insert_splits_original_and_reads_back() {
    let orig = b"abcdefghij";
    let mut buf = Buffer::new(&orig[..]);
    let ins = b"XXXXXXXXXXXXXXXXXXXX";
    buf.insert(5, ins).unwrap();
    let mut want = orig[..5].to_vec();
    want.extend_from_slice(&ins[..]);
    want.extend_from_slice(&orig[5..]);
    assert_eq!(buf.read(), want);
}

#[test]
fn insert_at_end_appends() {
    let mut buf = b("hello");
    buf.insert(5, b" world").unwrap();
    assert_eq!(buf.read(), b"hello world");
}

#[test]
fn insert_at_zero_prepends() {
    let mut buf = b("world");
    buf.insert(0, b"hello ").unwrap();
    assert_eq!(buf.read(), b"hello world");
}

#[test]
fn delete_excludes_range_and_undo_restores_exactly() {
    let mut buf = b("0123456789");
    buf.delete(3, 3).unwrap();
    assert_eq!(buf.read(), b"0126789");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"0123456789");
}

#[test]
fn delete_across_segment_boundaries() {
    let mut buf = b("0123456789");
    buf.insert(5, b"abc").unwrap();
    assert_eq!(buf.read(), b"01234abc56789");
    buf.delete(2, 6).unwrap();
    assert_eq!(buf.read(), b"0156789");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"01234abc56789");
}

#[test]
fn delete_to_the_end_of_document() {
    let mut buf = b("0123456789");
    buf.delete(7, 3).unwrap();
    assert_eq!(buf.read(), b"0123456");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"0123456789");
}

#[test]
fn delete_never_touches_deleted_again() {
    // Диапазон удаляется один раз. Повторный delete по тем же координатам
    // оригинала в live-координатах не существует: удаляется то, что на них.
    let mut buf = b("0123456789");
    buf.delete(2, 4).unwrap();
    assert_eq!(buf.read(), b"016789");
    buf.delete(2, 4).unwrap();
    assert_eq!(buf.read(), b"01");
}

#[test]
fn undo_insert_removes_text_and_redo_restores() {
    let mut buf = b("hello");
    buf.insert(5, b" world").unwrap();
    assert_eq!(buf.read(), b"hello world");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"hello");
    buf.redo().unwrap().unwrap();
    assert_eq!(buf.read(), b"hello world");
}

#[test]
fn undo_redo_walk_history_linearly() {
    let mut buf = b("");
    buf.insert(0, b"A").unwrap();
    buf.insert(1, b"B").unwrap();
    buf.insert(2, b"C").unwrap();
    assert_eq!(buf.read(), b"ABC");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"AB");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"A");
    buf.redo().unwrap().unwrap();
    assert_eq!(buf.read(), b"AB");
    buf.redo().unwrap().unwrap();
    assert_eq!(buf.read(), b"ABC");
}

#[test]
fn new_edit_after_undo_clears_redo() {
    let mut buf = b("");
    buf.insert(0, b"A").unwrap();
    buf.insert(1, b"B").unwrap();
    buf.undo().unwrap().unwrap();
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"");
    assert!(!buf.redo_stack().is_empty());
    buf.insert(0, b"X").unwrap();
    assert!(buf.redo_stack().is_empty());
    assert_eq!(buf.read(), b"X");
}

#[test]
fn replace_is_delete_then_insert() {
    let mut buf = b("Hello, world!");
    buf.replace(7, 5, b"Claxis").unwrap();
    assert_eq!(buf.read(), b"Hello, Claxis!");
    // replace = delete + insert: первая отмена убирает вставку.
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"Hello, !");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"Hello, world!");
}

#[test]
fn nested_inserts_resolve_to_flat_document() {
    let mut buf = b("");
    buf.insert(0, b"ace").unwrap();
    buf.insert(1, b"b").unwrap();
    buf.insert(3, b"d").unwrap();
    assert_eq!(buf.read(), b"abcde");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"abce");
}

#[test]
fn delete_undo_preserves_state_when_deleted_was_inserted_earlier() {
    let mut buf = b("0123456789");
    buf.insert(5, b"abc").unwrap();
    buf.delete(6, 1).unwrap();
    assert_eq!(buf.read(), b"01234ac56789");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"01234abc56789");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"0123456789");
}

#[test]
fn read_range_is_substring_of_full_read() {
    let mut buf = b("");
    for i in 0..100u8 {
        buf.insert(buf.len(), &[b'a' + (i % 26)]).unwrap();
    }
    let full = buf.read();
    assert_eq!(buf.read_range(20, 60), full[20..60]);
    assert!(buf.read_range(0, 0).is_empty());
    assert_eq!(buf.read_range(90, 200), full[90..]);
    assert_eq!(buf.read_range(0, 100), full);
}

#[test]
fn read_range_after_deletes_and_inserts() {
    let mut buf = b("0123456789");
    buf.insert(5, b"abc").unwrap();
    buf.delete(2, 3).unwrap();
    let full = buf.read();
    assert_eq!(buf.read_range(1, 8), full[1..8]);
}

#[test]
fn out_of_bounds_is_an_error() {
    let mut buf = b("abc");
    assert!(buf.insert(4, b"x").is_err());
    assert!(buf.delete(3, 1).is_err());
    assert!(buf.delete(100, 1).is_err());
}

#[test]
fn zero_length_ops_add_no_history() {
    let mut buf = b("abc");
    buf.insert(1, b"").unwrap();
    buf.delete(1, 0).unwrap();
    assert!(buf.undo_stack().is_empty());
}

#[test]
fn undo_and_redo_are_lifo_stacks() {
    let mut buf = b("");
    buf.insert(0, b"A").unwrap();
    buf.insert(1, b"B").unwrap();
    buf.insert(2, b"C").unwrap();
    assert_eq!(buf.undo_stack().len(), 3);
    let id = buf.undo().unwrap().unwrap();
    assert_eq!(buf.redo_stack().len(), 1);
    assert_eq!(buf.record(id).unwrap().anchor(), 2);
    assert_eq!(buf.record(id).unwrap().head(), 3);
}

#[test]
fn segments_hold_record_and_text_refs() {
    let mut buf = b("abcde");
    buf.insert(2, b"XY").unwrap();
    let segs = buf.segments();
    assert_eq!(segs.len(), 3);
    // Исходный текст без записи.
    assert_eq!(segs[0].record, crate::arena::ORIGINAL_ID);
    assert_eq!(segs[0].len, 2);
    // Вставка — своя запись, текст из Added.
    let insert = segs[1];
    assert_ne!(insert.record, crate::arena::ORIGINAL_ID);
    assert_eq!(insert.len, 2);
    assert_eq!(segs[2].len, 3);
}

#[test]
fn tree_stays_balanced_under_many_inserts() {
    // Дерево не должно вырождаться в список: высота обязана расти логарифмом.
    let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
    for i in 0..5_000u32 {
        buf.insert(i, b"x").unwrap();
    }
    let height = buf.tree_height();
    let leaves = buf.segments().len();
    assert!(
        leaves >= 5_000,
        "листьев должно быть не меньше 5000, got {leaves}"
    );
    // AVL-дерево из n листьев: высота <= 1.44*log2(n+2).
    let bound = 1.45 * (leaves as f64 + 2.0).log2();
    assert!(
        height as f64 <= bound,
        "высота {height} превышает границу {bound:.1} при {leaves} листьях"
    );
}

#[test]
fn tree_stays_balanced_under_deletes() {
    let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
    for _ in 0..5_000 {
        buf.insert(buf.len(), b"xy").unwrap();
    }
    // Удаляем каждый второй байт с начала — листья тают, баланс должен остаться.
    while !buf.is_empty() {
        buf.delete(0, 1).unwrap();
    }
    assert!(buf.is_empty());
    assert_eq!(buf.segments().len(), 0);
    assert_eq!(buf.tree_height(), 0);

    // Снова наполняем — высота обязана быть логарифмической.
    for _ in 0..5_000 {
        buf.insert(buf.len(), b"xy").unwrap();
    }
    let leaves = buf.segments().len();
    let height = buf.tree_height();
    let bound = 1.45 * (leaves as f64 + 2.0).log2();
    assert!(
        height as f64 <= bound,
        "высота {height} превышает границу {bound:.1} при {leaves} листьях"
    );
    assert_eq!(buf.read().len(), 10_000);
}

#[test]
fn node_pool_does_not_grow_without_bound() {
    // Освобождённые узлы идут в пул повторного использования: после волны
    // вставок и удалений размер пула заметно меньше суммарного числа операций.
    let mut buf = Buffer::with_history_depth(b"", 8_192).unwrap();
    for _ in 0..4_000 {
        buf.insert(buf.len(), b"ab").unwrap();
    }
    assert_eq!(buf.read().len(), 8_000);
    let after_inserts = buf.segments().len();

    // Удаляем всё, потом наполняем заново — пул переиспользуется, листья
    // не растут бесконечно.
    while !buf.is_empty() {
        buf.delete(0, 1).unwrap();
    }
    assert!(buf.segments().is_empty());
    for _ in 0..4_000 {
        buf.insert(buf.len(), b"ab").unwrap();
    }
    assert_eq!(buf.read().len(), 8_000);
    assert!(
        buf.segments().len() <= after_inserts,
        "листьев снова выросло: {} против {after_inserts}",
        buf.segments().len()
    );
}

#[test]
fn snapshot_moves_current_into_new_original() {
    let mut buf = b("hello");
    buf.insert(5, b" world").unwrap();
    buf.snapshot().unwrap();
    assert_eq!(buf.original(), b"hello world");
    assert_eq!(buf.read(), b"hello world");
    assert!(buf.undo_stack().is_empty());
    buf.insert(11, b"!").unwrap();
    assert_eq!(buf.read(), b"hello world!");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"hello world");
}

#[test]
fn snapshot_keeps_document_bounded_in_memory() {
    // Листья ничем не ограничены — ограничено число записей. Когда арена
    // исчерпала глубину, снапшот схлопывает дерево в новый Original. Текст при
    // этом не теряется.
    const DEPTH: u32 = 1_000;
    let mut buf = Buffer::with_history_depth(b"", DEPTH).unwrap();
    let mut peak_records = 0usize;
    for _ in 0..(DEPTH as usize * 3) {
        buf.insert(buf.len(), b"xy").unwrap();
        peak_records = peak_records.max(buf.undo_stack().len());
    }
    assert!(
        peak_records <= DEPTH as usize,
        "записей не должно стать больше глубины"
    );
    assert_eq!(
        peak_records,
        DEPTH as usize - 1,
        "снапшот берётся перед записью, поэтому в арене на одну запись меньше"
    );
    assert!(buf.snapshots_taken() > 0, "снапшот обязан был сработать");
    assert_eq!(buf.read().len(), DEPTH as usize * 6);

    // Правки продолжаются от нового исходного текста.
    buf.insert(buf.len(), b"!").unwrap();
    let text = buf.read();
    assert_eq!(text[text.len() - 1], b'!');
    assert_eq!(text.len(), DEPTH as usize * 6 + 1);
    assert!(
        text[..DEPTH as usize * 6]
            .iter()
            .all(|&b| b == b'x' || b == b'y')
    );
}

#[test]
fn segments_are_sixteen_bytes() {
    // Упаковка ради памяти: сегмент — ровно четыре u32, без хвоста выравнивания.
    assert_eq!(Buffer::segment_size(), 16);
    // Узел — тоже 16 байт: вид узла в старшем бите, дискриминант не нужен.
    assert_eq!(Buffer::node_size(), 16);
}

#[test]
fn redo_text_is_reused_not_deleted() {
    // Тексты отменённых вставок не удаляются: `Added` обрезается по хвосту,
    // и следующая вставка перезаписывает то же место. Проверяем, что новая
    // вставка действительно легла поверх отменённой, а не после неё.
    let mut buf = b("");
    buf.insert(0, b"AAAA").unwrap();
    buf.insert(4, b"BBBB").unwrap();
    buf.insert(8, b"CCCC").unwrap();
    buf.undo().unwrap().unwrap(); // отменена "CCCC"
    buf.undo().unwrap().unwrap(); // отменена "BBBB"
    assert_eq!(buf.read(), b"AAAA");

    // Новая вставка обязана занять место отменённых, а не дописать после.
    buf.insert(4, b"ZZ").unwrap();
    assert_eq!(buf.read(), b"AAAAZZ");
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"AAAA");
    buf.redo().unwrap().unwrap();
    assert_eq!(buf.read(), b"AAAAZZ");
}

#[test]
fn redo_text_reuse_shrinks_added_back() {
    // После отмены текст в `Added` остаётся: он нужен, если правка вернётся
    // redo. Обрезка случается только когда приходит новая правка — тогда
    // redo-ветка отменяется, а место переиспользуется.
    let mut buf = b("");
    for _ in 0..10 {
        buf.insert(buf.len(), b"0123456789").unwrap();
    }
    for _ in 0..10 {
        buf.undo().unwrap().unwrap();
    }
    assert!(buf.is_empty());
    assert!(
        !buf.redo_stack().is_empty(),
        "отменённые записи ждут в redo, текст из-за них не выбрасывается"
    );
    assert_eq!(
        buf.tree_memory().added_len,
        100,
        "текст отменённых вставок остаётся в Added ради redo"
    );

    // Новая правка: redo очищается, хвост срезается, место переиспользуется.
    let capacity_before = buf.tree_memory().added_capacity;
    buf.insert(0, b"x").unwrap();
    let mem = buf.tree_memory();
    assert_eq!(mem.added_len, 1, "хвост срезан до новой вставки");
    assert!(buf.redo_stack().is_empty());
    assert!(
        mem.added_capacity <= capacity_before,
        "ёмкость не должна расти: {} против {capacity_before}",
        mem.added_capacity
    );
    assert_eq!(buf.read(), b"x");
}

#[test]
fn arena_blocks_keep_all_records() {
    // Глубина ограничивает историю: записей не больше заданного, лишнее
    // сворачивается снапшотом, текст при этом цел. Цепочка блоков арены
    // проверяется отдельно, напрямую на арене — через буфер до неё не дойти.
    const DEPTH: u32 = 2_048;
    let mut buf = Buffer::with_history_depth(b"", DEPTH).unwrap();
    for _ in 0..3000 {
        buf.insert(0, b"X").unwrap();
        assert!(buf.undo_stack().len() <= DEPTH as usize);
        for &id in buf.undo_stack() {
            assert!(buf.record(id).is_some(), "запись пропала");
        }
    }
    assert_eq!(buf.read().len(), 3_000);
    assert!(buf.snapshots_taken() > 0);
    // Отменить можно только до последнего снапшота: дальше история уже стёрта,
    // поэтому исходный пустой документ недостижим.
    while buf.undo().unwrap().is_some() {}
    assert!(
        !buf.is_empty(),
        "откат должен упереться в снапшот, не в ноль"
    );
    assert!(
        buf.undo_stack().is_empty(),
        "откатились до конца своей истории"
    );
}

#[test]
fn history_depth_boundaries() {
    // Глубина задаётся числом записей, отдельного выбора размера арены нет.
    assert_eq!(Buffer::new(b"x").history_depth(), 8_192);
    assert_eq!(
        Buffer::with_history_depth(b"x", 1).unwrap().history_depth(),
        1
    );
    assert_eq!(
        Buffer::with_history_depth(b"x", 500)
            .unwrap()
            .history_depth(),
        500
    );
}

#[test]
fn delete_physically_removes_segments() {
    let mut buf = Buffer::with_history_depth(b"0123456789", 2_048).unwrap();
    buf.delete(2, 4).unwrap();
    // Из чтения ушёл диапазон, сегментов больше нет — только живые куски.
    let segs = buf.segments();
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].src(), crate::ORIGINAL);
    assert_eq!((segs[0].off, segs[0].len), (0, 2));
    assert_eq!((segs[1].off, segs[1].len), (6, 4));
    assert_eq!(buf.read(), b"016789");
}

#[test]
fn delete_cuts_middle_of_single_segment() {
    // Удаление из середины исходника: сегмент разрезается на два живых куска.
    let mut buf = Buffer::with_history_depth(b"0123456789", 2_048).unwrap();
    buf.delete(3, 3).unwrap();
    let segs = buf.segments();
    assert_eq!(segs.len(), 2);
    assert_eq!((segs[0].off, segs[0].len), (0, 3));
    assert_eq!((segs[1].off, segs[1].len), (6, 4));
    assert_eq!(buf.read(), b"0126789");
}

#[test]
fn delete_whole_document() {
    let mut buf = Buffer::with_history_depth(b"abc", 2_048).unwrap();
    buf.delete(0, 3).unwrap();
    assert!(buf.segments().is_empty());
    assert!(buf.is_empty());
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"abc");
}

#[test]
fn huge_insert_is_not_limited_by_arena() {
    let mut buf = b("start|end");
    let big = vec![b'x'; 1_000_000];
    buf.insert(5, &big).unwrap();
    assert_eq!(buf.len(), 1_000_009);
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"start|end");
}

#[test]
fn discarded_redo_texts_removed_from_arena() {
    let mut buf = b("");
    buf.insert(0, b"A").unwrap();
    buf.insert(1, b"B").unwrap();
    buf.undo().unwrap().unwrap();
    assert_eq!(buf.read(), b"A");
    let before = buf.undo_stack().len();
    buf.insert(1, b"C").unwrap();
    assert_eq!(buf.read(), b"AC");
    assert_eq!(buf.undo_stack().len(), before + 1);
    assert!(buf.redo_stack().is_empty());
}

#[test]
fn differential_against_reference_string() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    let mut rng = Rng(0x7f4a7c15);
    let mut buf = Buffer::new(b"");
    let mut ref_text = String::new();

    for _ in 0..3000 {
        let data = [b'a' + (rng.below(26) as u8)];
        if ref_text.is_empty() {
            buf.insert(0, &data).unwrap();
            ref_text.insert(0, data[0] as char);
            continue;
        }
        if rng.below(3) == 0 {
            // Удаление.
            let anchor = rng.below(ref_text.len());
            let len = rng.below(ref_text.len() - anchor);
            buf.delete(anchor as u32, len as u32).unwrap();
            ref_text.drain(anchor..anchor + len);
        } else {
            // Вставка.
            let anchor = rng.below(ref_text.len() + 1);
            buf.insert(anchor as u32, &data).unwrap();
            ref_text.insert(anchor, data[0] as char);
        }
        assert_eq!(buf.read(), ref_text.as_bytes(), "состояния разошлись");
    }
}

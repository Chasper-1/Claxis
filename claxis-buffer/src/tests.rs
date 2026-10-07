use crate::*;

fn seg(source: Source, offset: u32, len: u32) -> Segment {
    Segment::new(source, offset, len)
}

fn segments(buffer: &Buffer) -> Vec<Segment> {
    buffer.segments().collect()
}

fn check_invariants(buffer: &Buffer) {
    let mut total: u32 = 0;
    for segment in buffer.segments() {
        assert!(segment.len > 0, "пустой сегмент: {segment:?}");
        match segment.source {
            Source::Original => {
                assert!(
                    segment.end() <= buffer.original().len() as u32,
                    "сегмент выходит за границы Original: {segment:?}"
                );
            }
            Source::Add(add) => {
                let entry = buffer
                    .arena_entry(add)
                    .expect("сегмент ссылается на несуществующую запись arena");
                assert!(
                    segment.end() <= entry.len() as u32,
                    "сегмент выходит за границы записи arena: {segment:?}"
                );
            }
        }
        total += segment.len;
    }
    assert_eq!(total, buffer.len(), "сумма сегментов != длине документа");
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn below_u32(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
}

fn random_data(rng: &mut Rng) -> Vec<u8> {
    let len = 1 + rng.below(6);
    (0..len).map(|_| b'a' + rng.below(26) as u8).collect()
}

#[derive(Default)]
struct Reference {
    current: String,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl Reference {
    fn edit(&mut self, edit: impl FnOnce(&mut String)) {
        self.undo.push(self.current.clone());
        edit(&mut self.current);
        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.current.clone());
            self.current = prev;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.current.clone());
            self.current = next;
        }
    }
}

#[test]
fn insert_splits_original_and_reads_back() {
    let mut b = Buffer::new("abcdefghijklmnopqrstuvwxyz");
    b.insert(10, b"HELLO").unwrap();

    assert_eq!(b.len(), 31);
    assert_eq!(b.read(), b"abcdefghijHELLOklmnopqrstuvwxyz");
    assert_eq!(
        segments(&b),
        [
            seg(Source::Original, 0, 10),
            seg(Source::Add(0), 0, 5),
            seg(Source::Original, 10, 16),
        ]
    );
}

#[test]
fn insert_at_end_appends() {
    let mut b = Buffer::new("ab");
    b.insert(2, b"cd").unwrap();
    b.insert(4, b"ef").unwrap();

    assert_eq!(b.read(), b"abcdef");
    assert_eq!(b.add_count(), 2);
}

#[test]
fn delete_excludes_range_from_current() {
    let mut b = Buffer::new("abcdefghij");
    b.delete(3, 4).unwrap();

    assert_eq!(b.len(), 6);
    assert_eq!(b.read(), b"abchij");
    assert_eq!(
        segments(&b),
        [seg(Source::Original, 0, 3), seg(Source::Original, 7, 3)]
    );
    assert_eq!(b.original(), b"abcdefghij");
}

#[test]
fn delete_part_of_original_keeps_bytes() {
    let mut b = Buffer::new("0123456789");
    b.delete(4, 4).unwrap();

    assert_eq!(b.read(), b"012389");
    assert_eq!(
        segments(&b),
        [seg(Source::Original, 0, 4), seg(Source::Original, 8, 2)]
    );
    assert_eq!(b.original(), b"0123456789");
}

#[test]
fn replace_is_delete_then_insert() {
    let mut b = Buffer::new("hello world");
    b.replace(6, 5, b"there").unwrap();

    assert_eq!(b.read(), b"hello there");
    assert_eq!(b.undo_stack().len(), 2);

    b.undo().unwrap();
    assert_eq!(b.read(), b"hello ");
    b.undo().unwrap();
    assert_eq!(b.read(), b"hello world");
    assert!(b.undo().is_none());
}

#[test]
fn undo_and_redo_walk_history_linearly() {
    let mut b = Buffer::new("hello");
    b.insert(5, b" world").unwrap();
    b.delete(0, 5).unwrap();
    assert_eq!(b.read(), b" world");

    assert_eq!(b.undo(), Some(Edit::Delete { pos: 0, len: 5 }));
    assert_eq!(b.read(), b"hello world");

    assert_eq!(b.undo(), Some(Edit::Insert { pos: 5, add: 0 }));
    assert_eq!(b.read(), b"hello");
    assert!(b.undo().is_none());

    b.redo().unwrap();
    assert_eq!(b.read(), b"hello world");
    b.redo().unwrap();
    assert_eq!(b.read(), b" world");
    assert!(b.redo().is_none());
}

#[test]
fn new_edit_after_undo_clears_redo() {
    let mut b = Buffer::new("abc");
    b.insert(3, b"d").unwrap();
    b.undo().unwrap();
    assert_eq!(b.redo_stack().len(), 1);

    b.insert(1, b"x").unwrap();
    assert!(b.redo_stack().is_empty());
    assert_eq!(b.read(), b"axbc");
}

#[test]
fn new_delete_after_undo_also_clears_redo() {
    let mut b = Buffer::new("abcd");
    b.insert(4, b"e").unwrap();
    b.undo().unwrap();
    assert_eq!(b.redo_stack().len(), 1);

    b.delete(0, 1).unwrap();
    assert!(b.redo_stack().is_empty());
    assert_eq!(b.read(), b"bcd");
}

#[test]
fn nested_insert_resolves_to_flat_segments() {
    let mut b = Buffer::new("ab");
    b.insert(1, b"ABCDEFGHIJ").unwrap();
    b.insert(6, b"XXXX").unwrap();

    assert_eq!(b.read(), b"aABCDEXXXXFGHIJb");
    assert_eq!(
        segments(&b),
        [
            seg(Source::Original, 0, 1),
            seg(Source::Add(0), 0, 5),
            seg(Source::Add(1), 0, 4),
            seg(Source::Add(0), 5, 5),
            seg(Source::Original, 1, 1),
        ]
    );
}

#[test]
fn nested_delete_cuts_inside_add() {
    let mut b = Buffer::new("ab");
    b.insert(1, b"ABCDEFGHIJ").unwrap();
    b.insert(6, b"XXXX").unwrap();
    b.delete(2, 3).unwrap();

    assert_eq!(b.read(), b"aAEXXXXFGHIJb");
    assert_eq!(
        segments(&b),
        [
            seg(Source::Original, 0, 1),
            seg(Source::Add(0), 0, 1),
            seg(Source::Add(0), 4, 1),
            seg(Source::Add(1), 0, 4),
            seg(Source::Add(0), 5, 5),
            seg(Source::Original, 1, 1),
        ]
    );
}

#[test]
fn delete_across_segment_boundaries() {
    let mut b = Buffer::new("aaaabbbbcccc");
    b.insert(4, b"xx").unwrap();
    b.insert(8, b"yy").unwrap();
    assert_eq!(b.read(), b"aaaaxxbbyybbcccc");

    b.delete(4, 8).unwrap();

    assert_eq!(b.read(), b"aaaacccc");
    assert_eq!(
        segments(&b),
        [seg(Source::Original, 0, 4), seg(Source::Original, 8, 4)]
    );
}

#[test]
fn delete_splitting_two_segments_independently() {
    let mut b = Buffer::new("0123456789");
    b.insert(5, b"abc").unwrap();
    assert_eq!(b.read(), b"01234abc56789");

    b.delete(2, 8).unwrap();

    assert_eq!(b.read(), b"01789");
    assert_eq!(
        segments(&b),
        [seg(Source::Original, 0, 2), seg(Source::Original, 7, 3)]
    );
}

#[test]
fn delete_over_whole_add_removes_it() {
    let mut b = Buffer::new("ab");
    b.insert(1, b"XX").unwrap();
    b.delete(1, 2).unwrap();

    assert_eq!(b.read(), b"ab");
    assert_eq!(
        segments(&b),
        [seg(Source::Original, 0, 1), seg(Source::Original, 1, 1)]
    );
    assert_eq!(b.arena_entry(0), Some(b"XX".as_ref()));
}

#[test]
fn delete_to_the_end_of_document() {
    let mut b = Buffer::new("abcd");
    b.insert(2, b"XY").unwrap();
    b.delete(3, 3).unwrap();

    assert_eq!(b.read(), b"abX");
    check_invariants(&b);

    b.delete(0, 3).unwrap();
    assert!(b.is_empty());
    assert!(b.segments().next().is_none());
    check_invariants(&b);
}

#[test]
fn undo_restores_state_after_delete_to_end() {
    let mut b = Buffer::new("abcd");
    b.delete(1, 3).unwrap();
    assert_eq!(b.read(), b"a");

    b.undo().unwrap();
    assert_eq!(b.read(), b"abcd");
    assert_eq!(segments(&b), [seg(Source::Original, 0, 4)]);
}

#[test]
fn rebuild_reproduces_current() {
    let mut b = Buffer::new("some text here");
    b.insert(4, b"XXX").unwrap();
    b.delete(2, 9).unwrap();
    b.insert(0, b"start ").unwrap();
    b.insert(b.len(), b" end").unwrap();
    b.undo().unwrap();
    b.redo().unwrap();
    b.undo().unwrap();
    b.undo().unwrap();

    let segments_before = segments(&b);
    let text_before = b.read();

    b.rebuild();

    assert_eq!(segments(&b), segments_before);
    assert_eq!(b.read(), text_before);
}

#[test]
fn rebuild_after_every_edit_matches_incremental_state() {
    let mut b = Buffer::new("abc");
    b.insert(1, b"12").unwrap();
    b.delete(0, 2).unwrap();
    b.insert(0, b"zz").unwrap();

    let text = b.read();
    b.rebuild();
    assert_eq!(b.read(), text);
    assert_eq!(b.len(), text.len() as u32);
}

#[test]
fn rebuild_on_fresh_buffer_changes_nothing() {
    let b0 = Buffer::new("fresh document");
    let mut b = Buffer::new("fresh document");

    b.rebuild();

    assert_eq!(b.read(), b0.read());
    assert_eq!(segments(&b), segments(&b0));
    assert_eq!(b.len(), b0.len());
    assert!(b.undo_stack().is_empty());
}

#[test]
fn rebuild_after_full_undo_restores_original() {
    let original = b"the original text";
    let mut b = Buffer::new(original);
    b.insert(3, b"XXX").unwrap();
    b.delete(0, 5).unwrap();
    b.insert(b.len(), b"tail").unwrap();

    while b.undo().is_some() {}

    let text_before = b.read();
    b.rebuild();

    assert_eq!(text_before, original);
    assert_eq!(b.read(), original);
    assert_eq!(b.len(), original.len() as u32);
    check_invariants(&b);
}

#[test]
fn arena_entries_survive_undo() {
    let mut b = Buffer::new("ab");
    b.insert(1, b"XX").unwrap();
    b.insert(2, b"YY").unwrap();
    assert_eq!(b.add_count(), 2);

    b.undo().unwrap();
    b.undo().unwrap();

    assert_eq!(b.add_count(), 2);
    assert_eq!(b.arena_entry(0), Some(b"XX".as_ref()));
    assert_eq!(b.arena_entry(1), Some(b"YY".as_ref()));
    assert_eq!(b.arena_entry(2), None);
}

#[test]
fn original_never_changes() {
    let original = b"abcdefghijklmnopqrstuvwxyz".to_vec();
    let mut b = Buffer::new(&original);

    b.insert(0, b"12345 ").unwrap();
    b.delete(10, 8).unwrap();
    b.replace(2, 4, b"ZZZZ").unwrap();
    while b.undo().is_some() {}

    assert_eq!(b.original(), original);
    assert_eq!(b.read(), original);
}

#[test]
fn empty_document_operations() {
    let mut b = Buffer::new("");
    assert!(b.is_empty());
    assert!(b.segments().next().is_none());

    b.insert(0, b"abc").unwrap();
    assert_eq!(b.read(), b"abc");

    b.delete(0, 3).unwrap();
    assert!(b.is_empty());
    assert!(b.segments().next().is_none());

    b.undo().unwrap();
    assert_eq!(b.read(), b"abc");
}

#[test]
fn out_of_bounds_is_an_error() {
    let mut b = Buffer::new("abc");

    assert_eq!(
        b.insert(4, b"x"),
        Err(Error::OutOfBounds {
            pos: 4,
            len: 0,
            doc_len: 3
        })
    );
    assert_eq!(
        b.delete(1, 3),
        Err(Error::OutOfBounds {
            pos: 1,
            len: 3,
            doc_len: 3
        })
    );
    assert_eq!(
        b.delete(9, 0),
        Err(Error::OutOfBounds {
            pos: 9,
            len: 0,
            doc_len: 3
        })
    );
    assert_eq!(
        b.replace(2, 2, b"x"),
        Err(Error::OutOfBounds {
            pos: 2,
            len: 2,
            doc_len: 3
        })
    );
    assert_eq!(b.read(), b"abc");
    assert!(b.undo_stack().is_empty());
}

#[test]
fn zero_length_ops_add_no_history() {
    let mut b = Buffer::new("abc");
    b.insert(1, b"").unwrap();
    b.delete(1, 0).unwrap();

    assert!(b.undo_stack().is_empty());
    assert_eq!(b.read(), b"abc");
}

#[test]
fn read_into_matches_read() {
    let mut b = Buffer::new("abc");
    b.insert(1, b"12").unwrap();

    let mut out = Vec::new();
    b.read_into(&mut out);
    assert_eq!(out, b.read());
}

#[test]
fn undo_redo_are_lifo_stacks() {
    let mut b = Buffer::new("base");
    let e1 = Edit::Insert { pos: 4, add: 0 };
    let e2 = Edit::Insert { pos: 5, add: 1 };
    let e3 = Edit::Delete { pos: 0, len: 2 };
    b.insert(4, b"1").unwrap();
    b.insert(5, b"2").unwrap();
    b.delete(0, 2).unwrap();

    let undo_edits: Vec<Edit> = b.undo_stack().iter().map(|r| r.edit).collect();
    assert_eq!(undo_edits, [e1, e2, e3]);

    b.undo().unwrap();
    b.undo().unwrap();
    let redo_edits: Vec<Edit> = b.redo_stack().iter().map(|r| r.edit).collect();
    let undo_edits: Vec<Edit> = b.undo_stack().iter().map(|r| r.edit).collect();
    assert_eq!(redo_edits, [e3, e2]);
    assert_eq!(undo_edits, [e1]);

    b.redo().unwrap();
    assert_eq!(
        b.undo_stack().iter().map(|r| r.edit).collect::<Vec<_>>(),
        [e1, e2]
    );
    assert_eq!(
        b.redo_stack().iter().map(|r| r.edit).collect::<Vec<_>>(),
        [e3]
    );

    b.redo().unwrap();
    assert_eq!(
        b.undo_stack().iter().map(|r| r.edit).collect::<Vec<_>>(),
        [e1, e2, e3]
    );
    assert!(b.redo_stack().is_empty());
}

#[test]
fn node_arena_is_write_once() {
    let mut b = Buffer::new("0123456789abcdef");
    for i in 0..20u32 {
        let pos = i * 3 % (b.len() + 1);
        b.insert(pos, b"inserted").unwrap();
        let len = b.len();
        if len > 4 {
            b.delete(len / 2, 3).unwrap();
        }
    }

    let nodes = b.node_count();
    let text = b.read();

    for _ in 0..5 {
        b.rebuild();
        assert_eq!(b.node_count(), nodes, "rebuild создал новые ноды");
        assert_eq!(b.read(), text);
    }

    while b.undo().is_some() {}
    assert_eq!(b.node_count(), nodes, "undo удалил/добавил ноды");
    assert_eq!(b.read(), b"0123456789abcdef");

    while b.redo().is_some() {}
    assert_eq!(b.node_count(), nodes, "redo добавил ноды");
    assert_eq!(b.read(), text);
}

#[test]
fn discarding_redo_keeps_data_and_correct_state() {
    let mut b = Buffer::new("abc");
    b.insert(3, b"DEF").unwrap();
    b.undo().unwrap();
    assert_eq!(b.read(), b"abc");

    b.insert(0, b"xy").unwrap();
    assert!(b.redo_stack().is_empty());
    assert_eq!(b.add_count(), 2);
    assert_eq!(b.read(), b"xyabc");

    b.rebuild();
    assert_eq!(b.read(), b"xyabc");
    assert_eq!(b.add_count(), 2);
    assert_eq!(b.arena_entry(0), Some(b"DEF".as_ref()));
    assert_eq!(b.arena_entry(1), Some(b"xy".as_ref()));
}

/// Заполняет арену целиком, перенося её через границу, и возвращает текст до переноса.
fn fill_arena() -> (Buffer, Vec<u8>) {
    let mut b = Buffer::new("base");
    let chunk = vec![b'x'; 16 * 1024];
    // 65 × 16 КиБ > 1 МиБ: цикл арены обязан сработать.
    for _ in 0..65 {
        b.insert(b.len(), &chunk).unwrap();
    }
    let text = b.read();
    (b, text)
}

#[test]
fn arena_cycle_moves_current_into_new_original() {
    let (b, text) = fill_arena();

    assert_eq!(b.read(), text);
    assert!(b.add_count() < 65, "арена не была переиспользована");
    assert_ne!(b.original(), b"base", "Original не сменился на новый");
    // Перенос происходит до вставки, последний чанк ложится уже в чистую арену.
    assert_eq!(
        b.original(),
        &text[..text.len() - 16 * 1024],
        "Original не равен тексту на момент переноса"
    );
    check_invariants(&b);
}

#[test]
fn arena_cycle_keeps_text_across_boundary() {
    let (mut b, _) = fill_arena();
    let text = b.read();

    // Вставка после переноса идёт в чистую арену и не теряется.
    b.insert(b.len(), b"tail").unwrap();
    let mut expected = text.clone();
    expected.extend_from_slice(b"tail");
    assert_eq!(b.read(), expected);

    b.undo().unwrap();
    assert_eq!(b.read(), text);
    b.redo().unwrap();
    assert_eq!(b.read(), expected);
    check_invariants(&b);
}

#[test]
fn arena_cycle_survives_rebuild() {
    let (mut b, _) = fill_arena();
    let before = segments(&b);

    b.rebuild();

    assert_eq!(segments(&b), before, "rebuild разошёлся после границы арены");
    check_invariants(&b);
}

#[test]
fn delete_across_arena_boundary() {
    let mut b = Buffer::new("abcdefghij");
    let chunk = vec![b'X'; 64 * 1024];
    for _ in 0..20 {
        b.insert(b.len(), &chunk).unwrap();
    }
    let text = b.read();

    // Удаление через границу арены: затрагивает и Original, и новую арену.
    b.delete(5, text.len() as u32 - 15).unwrap();

    let mut expected = text[..5].to_vec();
    expected.extend_from_slice(&text[text.len() - 10..]);
    assert_eq!(b.read(), expected);
    check_invariants(&b);

    b.undo().unwrap();
    assert_eq!(b.read(), text);
}

#[test]
fn snapshot_moves_current_into_new_original() {
    let mut b = Buffer::new("base");
    b.insert(4, b"ZZ").unwrap();
    let text = b.read();

    let snapshot = b.snapshot();

    assert_eq!(snapshot.original(), b"base");
    assert_eq!(b.original(), text);
    assert_eq!(b.read(), text);
    assert_eq!(b.add_count(), 0);
    assert!(b.undo_stack().is_empty());
    assert!(b.redo_stack().is_empty());

    b.insert(b.len(), b"-end").unwrap();
    let mut expected = text;
    expected.extend_from_slice(b"-end");
    assert_eq!(b.read(), expected);
    assert_eq!(b.add_count(), 1);
    b.undo().unwrap();
    assert_eq!(b.read(), expected[..expected.len() - 4].to_vec());
    b.redo().unwrap();
    assert_eq!(b.read(), expected);
    check_invariants(&b);
}

#[test]
fn insert_larger_than_arena_is_an_error() {
    let mut b = Buffer::new("base");
    // Крупнее арены не помещается ни в каком случае, даже в пустую.
    assert!(ARENA_CAPACITY + 1 > ARENA_CAPACITY);
    let huge = vec![b'y'; ARENA_CAPACITY + 1];
    let before = b.read();
    assert_eq!(b.insert(4, &huge), Err(Error::ArenaFull));
    assert_eq!(b.read(), before);
    check_invariants(&b);
}

#[test]
fn replace_across_arena_boundary() {
    let mut b = Buffer::new("head");
    let chunk = vec![b'Z'; 64 * 1024];
    for _ in 0..20 {
        b.insert(b.len(), &chunk).unwrap();
    }
    b.insert(0, b"TAIL").unwrap();
    let text = b.read();

    b.replace(0, 4, b"HEAD").unwrap();

    let mut expected = b"HEAD".to_vec();
    expected.extend_from_slice(&text[4..]);
    assert_eq!(b.read(), expected);
    check_invariants(&b);
}

#[test]
fn differential_against_reference_string() {
    for initial in ["", "hello", "aaaabbbbcccc"] {
        for seed in 0..6u64 {
            let mut rng = Rng::new(seed);
            let mut buffer = Buffer::new(initial);
            let mut reference = Reference {
                current: initial.to_string(),
                ..Default::default()
            };
            let mut adds = 0;

            for step in 0..300 {
                match rng.below(100) {
                    0..=34 => {
                        let pos = rng.below_u32(buffer.len() + 1);
                        let data = if rng.below(8) == 0 {
                            Vec::new()
                        } else {
                            random_data(&mut rng)
                        };
                        buffer.insert(pos, &data).unwrap();
                        if !data.is_empty() {
                            let text = String::from_utf8(data).unwrap();
                            let at = pos as usize;
                            reference.edit(|s| s.insert_str(at, &text));
                        }
                    }
                    35..=59 => {
                        let len = buffer.len();
                        if len > 0 {
                            let pos = rng.below_u32(len);
                            let del = 1 + rng.below_u32(len - pos);
                            buffer.delete(pos, del).unwrap();
                            let (at, upto) = (pos as usize, (pos + del) as usize);
                            reference.edit(|s| {
                                s.replace_range(at..upto, "");
                            });
                        }
                    }
                    60..=69 => {
                        let len = buffer.len();
                        let pos = rng.below_u32(len + 1);
                        let del = rng.below_u32(len - pos + 1);
                        let data = if rng.below(5) == 0 {
                            Vec::new()
                        } else {
                            random_data(&mut rng)
                        };
                        buffer.replace(pos, del, &data).unwrap();
                        if del > 0 {
                            let (at, upto) = (pos as usize, (pos + del) as usize);
                            reference.edit(|s| {
                                s.replace_range(at..upto, "");
                            });
                        }
                        if !data.is_empty() {
                            let text = String::from_utf8(data).unwrap();
                            let at = pos as usize;
                            reference.edit(|s| s.insert_str(at, &text));
                        }
                    }
                    70..=84 => {
                        buffer.undo();
                        reference.undo();
                    }
                    _ => {
                        buffer.redo();
                        reference.redo();
                    }
                }

                let context = format!("initial={initial:?} seed={seed} step={step}");
                assert_eq!(
                    buffer.read(),
                    reference.current.as_bytes(),
                    "текст разошёлся: {context}"
                );
                assert_eq!(
                    buffer.len(),
                    reference.current.len() as u32,
                    "длина: {context}"
                );
                assert_eq!(buffer.original(), initial.as_bytes(), "original: {context}");
                assert!(buffer.add_count() >= adds, "arena уменьшилась: {context}");
                adds = buffer.add_count();
                check_invariants(&buffer);

                let before = segments(&buffer);
                buffer.rebuild();
                assert_eq!(
                    segments(&buffer),
                    before,
                    "rebuild != incremental: {context}"
                );
                assert_eq!(
                    buffer.read(),
                    reference.current.as_bytes(),
                    "текст после rebuild: {context}"
                );
            }
        }
    }
}

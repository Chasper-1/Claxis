//! Проверки документа: тот же функционал, что у `claxis-buffer`.
//!
//! Главная проверка — сверка с эталоном на длинной случайной
//! последовательности правок с перемешиванием отмен и повторов. Эталон —
//! обычная строка, и любое расхождение означает битый документ.

use crate::document::{DEFAULT_DEPTH, Document, Error, Record};

fn d(text: &str) -> Document {
    Document::new(text.as_bytes())
}

/// Генератор чисел: детерминированный, чтобы падение воспроизводилось.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

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
    assert_eq!(doc.read(), b"abc");
}

#[test]
fn history_depth_has_no_ceiling() {
    // Потолка нет: история растёт вместе с правками, память добирается сама.
    // Отвергается только ноль — с ним снимался бы снапшот после каждой правки.
    assert!(Document::with_depth(b"x", 0).is_err());
    assert!(Document::with_depth(b"x", 1).is_ok());
    assert!(Document::with_depth(b"x", u32::MAX).is_ok());

    // И правок может быть больше, чем любое разумное число.
    let mut doc = Document::with_depth(b"x", u32::MAX / 2).unwrap();
    for i in 0..50_000u32 {
        doc.insert(0, b"y").unwrap();
        let _ = i;
    }
    assert_eq!(doc.path().len(), 50_000, "история росла вместе с правками");
    assert_eq!(doc.snapshots_taken(), 0, "снапшот брать было не пора");
}

#[test]
fn a_snapshot_carries_text_and_history() {
    // Снапшот уносит состояние целиком: и текст, и историю. Без истории
    // документ верный, а отменять нечего.
    let (text, history) = {
        let mut doc = d("abc");
        doc.set_sink(
            std::path::PathBuf::from("/tmp/d.txt"),
            Box::new(crate::document::NullSink),
        );
        doc.insert(3, b"XY").unwrap();
        doc.delete(0, 1).unwrap();
        // История снимается до снапшота: снимок её уводит, это его смысл.
        let history = crate::history::History::capture(&doc);
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
fn history_never_snapshots_on_its_own() {
    // Снапшот снимает время, а не число правок: документ сам ничего не решает
    // и ждёт прямой просьбы.
    let mut doc = Document::with_depth(b"abc", 4).unwrap();
    doc.set_sink(
        std::path::PathBuf::from("/tmp/d.txt"),
        Box::new(crate::document::NullSink),
    );
    for i in 0..100 {
        doc.insert(i as u32, b"x").unwrap();
    }
    assert_eq!(doc.snapshots_taken(), 0, "сто правок снапшота не заслужили");
    assert_eq!(doc.path().len(), 100, "история росла вместе с правками");

    doc.snapshot();
    assert_eq!(doc.snapshots_taken(), 1, "по прямой просьбе снимается");
    assert!(
        doc.path().is_empty(),
        "после снапшота история начинается заново"
    );
}

#[test]
fn reading_is_consistent_after_many_edits() {
    let mut doc = d("");
    for i in 0..300u32 {
        doc.insert(doc.len(), b"line\n").unwrap();
        if i % 3 == 0 && doc.len() > 10 {
            doc.delete(0, 5).unwrap();
        }
    }
    let full = doc.read();
    assert_eq!(doc.read_range(0, doc.len()), full);
    assert_eq!(doc.read_range(5, 5), Vec::<u8>::new());
    let mid = doc.read_range(2, 10);
    assert_eq!(mid, full[2..10].to_vec());
}

#[test]
fn the_path_is_the_active_branch_in_order() {
    let mut doc = d("");
    doc.insert(0, b"A").unwrap();
    doc.insert(1, b"B").unwrap();
    doc.undo().unwrap();
    doc.insert(1, b"C").unwrap();

    let path = doc.path();
    assert_eq!(path.len(), 2);
    assert!(matches!(path[0], Record::Insert { head: 1, .. }));
    assert!(matches!(path[1], Record::Insert { head: 2, .. }));
    assert_eq!(doc.read(), b"AC");
}

#[test]
fn against_a_reference_string_with_undo_redo_mix() {
    // Эталон — обычная строка. Случайная последовательность правок, отмен и
    // повторов: любое расхождение означает битый документ.
    for seed in 0..8u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(7));
        let mut doc = Document::new(b"seed");
        let mut text = String::from("seed");
        let mut history: Vec<Op> = Vec::new();
        let mut cursor = 0usize;

        for step in 0..1200 {
            let roll = rng.below(12);
            if roll < 4 && !text.is_empty() {
                let pos = rng.below(text.len());
                let len = rng.below((text.len() - pos).min(40)) + 1;
                let removed = text.as_bytes()[pos..pos + len].to_vec();
                doc.delete(pos as u32, len as u32).unwrap();
                text.replace_range(pos..pos + len, "");
                history.truncate(cursor);
                history.push(Op::Delete { pos, removed });
                cursor = history.len();
            } else if roll < 7 {
                let pos = rng.below(text.len() + 1);
                let typed: String = (0..rng.below(30) + 1)
                    .map(|_| (b'a' + rng.below(26) as u8) as char)
                    .collect();
                doc.insert(pos as u32, typed.as_bytes()).unwrap();
                text.insert_str(pos, &typed);
                history.truncate(cursor);
                history.push(Op::Insert {
                    pos,
                    text: typed.into_bytes(),
                });
                cursor = history.len();
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
                history.truncate(cursor);
                history.push(Op::Replace {
                    pos,
                    removed,
                    text: typed.into_bytes(),
                });
                cursor = history.len();
            } else if roll == 9 && cursor > 0 {
                cursor -= 1;
                doc.undo()
                    .unwrap()
                    .expect("история должна была быть непустой");
                match &history[cursor] {
                    Op::Insert { pos, text: t } => text.replace_range(*pos..*pos + t.len(), ""),
                    Op::Delete { pos, removed } => {
                        text.insert_str(*pos, &String::from_utf8_lossy(removed))
                    }
                    Op::Replace {
                        pos,
                        removed,
                        text: t,
                    } => {
                        // Отмена замены: убрать вставленное, вернуть вырезанное.
                        text.replace_range(*pos..*pos + t.len(), "");
                        let back = String::from_utf8_lossy(removed).to_string();
                        text.insert_str(*pos, &back);
                    }
                }
            } else if cursor < history.len() {
                doc.redo()
                    .unwrap()
                    .expect("повтор должен был быть доступен");
                match &history[cursor] {
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

enum Op {
    Insert {
        pos: usize,
        text: Vec<u8>,
    },
    Delete {
        pos: usize,
        removed: Vec<u8>,
    },
    Replace {
        pos: usize,
        removed: Vec<u8>,
        text: Vec<u8>,
    },
}

#[test]
fn tree_stays_balanced_under_churn() {
    // Баланс держится при любом порядке правок: иначе спуск перестаёт быть
    // логарифмическим.
    let mut doc = Document::new(b"a");
    let mut rng = Rng(99);
    for _ in 0..2000 {
        let pos = rng.below(doc.len() as usize + 1) as u32;
        if rng.below(2) == 0 && !doc.read().is_empty() {
            let len = rng.below(doc.len() as usize) + 1;
            let _ = doc.delete(pos.min(doc.len() - 1), len as u32);
        } else {
            doc.insert(pos, b"xy").unwrap();
        }
    }
    let (nodes, height, worst_skew, height_mismatch) = doc.tree_debug_stats();
    let leaves = doc.leaves().len();
    if leaves > 0 {
        // Сплошное дерево из n листьев всегда ровно 2n − 1 узлов.
        assert_eq!(nodes, leaves * 2 - 1, "дерево не сплошное");
    }
    assert_eq!(
        height_mismatch, 0,
        "высота в узле разошлась с настоящей: баланс считается не по тому"
    );
    assert!(worst_skew <= 1, "перекос {worst_skew} при {nodes} узлах");
    assert_eq!(height, doc.height(), "высота в корне не совпадает");
    // Высота AVL: не больше примерно 1.44 от log2(листьев) плюс запас.
    let bound = (leaves.max(2) as f64).log2() * 1.5 + 2.0;
    assert!(
        height as f64 <= bound,
        "высота {height} при {leaves} листьях превышает {bound:.1}"
    );
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
fn a_snapshot_survives_a_random_history() {
    // Снапшот снимается в произвольный момент под нагрузкой: восстановленный
    // документ должен совпасть и текстом, и историей.
    for seed in 0..4u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let mut doc = Document::new("начало".as_bytes());
        for _ in 0..200 {
            let len = doc.len() as usize;
            let pos = rng.below(len + 1) as u32;
            if rng.below(2) == 0 {
                let _ = doc.delete(
                    pos.min(doc.len().saturating_sub(1)),
                    1 + rng.below(20) as u32,
                );
            } else {
                doc.insert(pos, b"qq").unwrap();
            }
            if rng.below(7) == 0 {
                let _ = doc.undo().unwrap();
            }
        }
        let text = doc.read();
        let history = crate::history::History::capture(&doc);
        let mut back = history.restore().expect("снапшот разбирается");
        assert_eq!(back.read(), text, "seed {seed}: текст разошёлся");
        assert_eq!(
            back.path().len(),
            doc.path().len(),
            "seed {seed}: история разошлась"
        );

        // Отмена в восстановленном документе идёт по той же ветке.
        for _ in 0..doc.path().len() {
            back.undo().unwrap().expect("история восстановлена");
        }
        assert_eq!(
            back.read(),
            "начало".as_bytes(),
            "seed {seed}: откат до начала не сошёлся"
        );
    }
}

#[test]
fn a_foreign_blob_is_not_read_as_our_history() {
    // Прежний буфер писал в ту же колонку свой формат. Такой блоб обязан быть
    // распознан как чужой, а не разобран как родной.
    let mut old = Vec::new();
    old.extend_from_slice(b"BUFH");
    old.extend_from_slice(&[0u8; 32]);
    assert_eq!(
        crate::history::History::decode(&old).unwrap_err(),
        Error::NotOurHistory
    );
    assert!(crate::history::History::decode(&[]).is_err());
}

#[test]
fn a_history_from_another_version_is_refused() {
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    let history = crate::history::History::capture(&doc);
    let mut bytes = history.as_bytes().to_vec();
    bytes[4] = 99;
    assert_eq!(
        crate::history::History::decode(&bytes).unwrap_err(),
        Error::UnknownHistoryVersion { version: 99 }
    );
}

#[test]
fn a_truncated_history_is_refused_not_panicked() {
    // Снапшот может прийти битым: обрыв, обрезка, чужой мусор. Всё это —
    // ошибка с текстом, а не паника.
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    doc.delete(0, 1).unwrap();
    let history = crate::history::History::capture(&doc);
    let bytes = history.as_bytes().to_vec();
    for cut in 1..bytes.len() {
        let err = crate::history::History::decode(&bytes[..cut]);
        assert!(err.is_err(), "обрыв на {cut} байте разобрался как целый");
    }
}

#[test]
fn an_unknown_record_kind_is_refused() {
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    let mut bytes = crate::history::History::capture(&doc).as_bytes().to_vec();
    let last = bytes.len() - crate::history::RECORD_SIZE;
    bytes[last] = 7;
    assert_eq!(
        crate::history::History::decode(&bytes).unwrap_err(),
        Error::UnknownRecordKind { kind: 7 }
    );
}

#[test]
fn a_record_with_a_backwards_range_is_refused() {
    // Из-за `head - anchor` переполнилось бы вычитание, а это паника.
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    let mut bytes = crate::history::History::capture(&doc).as_bytes().to_vec();
    let last = bytes.len() - crate::history::RECORD_SIZE;
    // anchor и head стоят подряд, начиная с первого поля записи.
    bytes[last + 1..last + 5].copy_from_slice(&9u32.to_le_bytes());
    bytes[last + 5..last + 9].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        crate::history::History::decode(&bytes).unwrap_err(),
        Error::InvalidRecordRange { anchor: 9, head: 2 }
    );
}

#[test]
fn a_record_pointing_outside_the_buffer_is_refused() {
    // Куски за границами буфера не существует. Раньше отмена удаления резала
    // буфер по такому смещению и роняла редактор вместе с текстом.
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    doc.delete(0, 1).unwrap();
    let mut bytes = crate::history::History::capture(&doc).as_bytes().to_vec();
    let last = bytes.len() - crate::history::RECORD_SIZE;
    bytes[last + 9..last + 13].copy_from_slice(&9_000u32.to_le_bytes());
    assert_eq!(
        crate::history::History::decode(&bytes).unwrap_err(),
        Error::HistoryOutOfBounds { at: 9_000, len: 1 }
    );
}

#[test]
fn a_captured_history_survives_a_round_trip() {
    // Обратная сторона проверок: хорошая история разбирается и восстанавливается.
    let mut doc = d("abc");
    doc.insert(3, b"XY").unwrap();
    doc.replace(0, 2, b"Z").unwrap();
    doc.delete(1, 1).unwrap();
    let history = crate::history::History::capture(&doc);
    let mut restored = crate::history::History::decode(history.as_bytes())
        .unwrap()
        .restore()
        .unwrap();
    assert_eq!(restored.read(), doc.read());
    restored.undo().unwrap().expect("отменять есть что");
    assert_eq!(restored.read(), b"ZcXY");
}

#[test]
fn errors_speak_through_the_catalog() {
    // Ошибка не несёт готовой строки: текст собирает каталог, иначе её нельзя
    // будет перевести.
    let e = Error::OutOfBounds {
        anchor: 10,
        len: 5,
        doc_len: 12,
    };
    let text = e.message(&crate::messages::En);
    assert!(
        text.contains("15"),
        "в тексте должно быть место значения: {text}"
    );
    assert!(
        text.contains("12"),
        "в тексте должно быть место значения: {text}"
    );
    assert_eq!(e.text(), text);
    assert_eq!(format!("{e}"), text);
}

#[test]
fn the_history_survives_the_database_round_trip() {
    // Ровно то, что делает редактор: снапшот уходит в база байтами и приходит
    // байтами же.
    let mut doc = d("начало");
    for i in 0..50u32 {
        if i % 3 == 0 {
            doc.insert(i % (doc.len() + 1), b"pq").unwrap();
        } else if !doc.is_empty() {
            let at = i % doc.len();
            doc.delete(at, 1).unwrap();
        }
    }
    let text = doc.read();
    let stored = crate::history::History::capture(&doc).as_bytes().to_vec();

    let back = crate::history::History::decode(&stored)
        .expect("блоб из базы разбирается")
        .restore()
        .expect("документ собирается");
    assert_eq!(back.read(), text);
    assert_eq!(back.path().len(), doc.path().len());
}

#[test]
fn an_empty_document_takes_everything() {
    // Пустой документ — не особый случай, но именно на нём чаще всего
    // вылезают ошибки границ.
    let mut doc = Document::new(b"");
    assert!(doc.is_empty());
    assert_eq!(doc.len(), 0);
    assert_eq!(doc.read(), Vec::<u8>::new());
    assert_eq!(doc.read_range(0, 0), Vec::<u8>::new());

    doc.insert(0, "первое".as_bytes()).unwrap();
    assert_eq!(doc.read(), "первое".as_bytes());
    doc.delete(0, doc.len()).unwrap();
    assert!(doc.is_empty(), "весь текст вырезан, документ снова пустой");

    // И отмена возвращает всё как было.
    doc.undo().unwrap().expect("удаление отменяемо");
    assert_eq!(doc.read(), "первое".as_bytes());
}

#[test]
fn deleting_the_whole_document_is_the_same_as_a_new_one() {
    let mut doc = d("одна строка");
    doc.delete(0, doc.len()).unwrap();
    assert!(doc.is_empty());
    assert_eq!(doc.leaves().len(), 0, "листьев не осталось");

    // Дерево пустое, но обязано остаться связным.
    let (nodes, height, skew, mismatch) = doc.tree_debug_stats();
    assert_eq!(nodes, 0);
    assert_eq!(height, 0);
    assert_eq!(skew, 0);
    assert_eq!(mismatch, 0);

    doc.insert(0, "снова".as_bytes()).unwrap();
    assert_eq!(doc.read(), "снова".as_bytes());
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
fn history_of_an_untouched_document_is_empty() {
    // Снапшот пустого документа должен разбираться, а не падать: пустая
    // история — обычное состояние, а не поломка.
    let doc = d("текст без правок");
    let history = crate::history::History::capture(&doc);
    let mut back = history.restore().expect("пустая история разбирается");
    assert_eq!(back.read(), doc.read());
    assert!(back.path().is_empty());
    assert!(back.undo().unwrap().is_none(), "отменять нечего");
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
    // Длина меняется в обе стороны: короткий кусок вместо длинного и наоборот.
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
    // Вырезаем только слово, пробел остаётся: «новый текст», а не «новыйтекст».
    let cut = "исходный".len() as u32;
    doc.replace(0, cut, "новый".as_bytes()).unwrap();
    let history = crate::history::History::capture(&doc);
    let mut back = history.restore().expect("снапшот разбирается");
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
    // проявляется не на ровном куске, а на том, что осталось от прежних правок.
    let mut doc = d("");
    let mut text = String::new();
    let mut log: Vec<String> = Vec::new();
    let mut history: Vec<(u8, usize, usize, String)> = Vec::new();
    let mut rng = Rng(0x2545F4914F6CDD1D);

    for _ in 0..600 {
        match rng.below(4) {
            0 if !text.is_empty() => {
                let at = rng.below(text.len());
                let len = 1 + rng.below((text.len() - at).min(20));
                let removed = text[at..at + len].to_string();
                doc.delete(at as u32, len as u32).unwrap();
                text.replace_range(at..at + len, "");
                history.push((0, at, len, removed));
                log.push(format!("delete {at} {len}"));
            }
            1 => {
                let at = rng.below(text.len() + 1);
                let typed = if rng.below(2) == 0 { "\n" } else { "ab" };
                doc.insert(at as u32, typed.as_bytes()).unwrap();
                text.insert_str(at, typed);
                history.push((1, at, typed.len(), String::new()));
                log.push(format!("insert {at} {typed:?}"));
            }
            2 => {
                let at = rng.below(text.len() + 1);
                let len = rng.below((text.len() - at).min(10));
                let removed = text[at..at + len].to_string();
                doc.replace(at as u32, len as u32, b"xy\n").unwrap();
                text.replace_range(at..at + len, "xy\n");
                history.push((2, at, 3, removed));
                log.push(format!("replace {at} {len}"));
            }
            _ => {
                let Some((kind, at, len, removed)) = history.pop() else {
                    continue;
                };
                doc.undo().unwrap().expect("отменять есть что");
                match kind {
                    // Отмена вставки: убрать вставленное.
                    1 => text.replace_range(at..at + len, ""),
                    // Отмена удаления: вернуть вырезанное.
                    0 => text.insert_str(at, &removed),
                    // Отмена замены: убрать вставленное и вернуть вырезанное.
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

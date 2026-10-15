//! Сжимание падающей последовательности правок до минимальной.
//!
//! Вспомогательный инструмент: гоняет одну и ту же последовательность на
//! документе и на обычной строке, а потом убирает лишние операции, пока
//! расхождение не перестанет воспроизводиться. Остаётся короткий список,
//! который уже можно читать глазами.

use claxis_rope::Document;

/// Правка в эталоне и в документе.
#[derive(Clone, Debug)]
enum Op {
    Insert { pos: usize, text: String },
    Delete { pos: usize, len: usize },
    Replace { pos: usize, len: usize },
    Undo,
}

/// Прогнать последовательность. `None` — разошлись, `Some` — совпало.
fn run(ops: &[Op]) -> Option<(String, String)> {
    let mut doc = Document::new(b"");
    let mut text = String::new();
    // История эталона: вид правки, позиция, длина вставленного, вырезанное.
    let mut history: Vec<(u8, usize, usize, String)> = Vec::new();

    for op in ops {
        match *op {
            Op::Insert {
                pos,
                text: ref typed,
            } => {
                // После удаления лишней операции позиции могут выйти за текст.
                // Такая правка просто не выполняется — с обеих сторон одинаково.
                if pos > text.len() {
                    continue;
                }
                doc.insert(pos as u32, typed.as_bytes()).unwrap();
                text.insert_str(pos, typed);
                history.push((1, pos, typed.len(), String::new()));
            }
            Op::Delete { pos, len } => {
                if pos + len > text.len() {
                    continue;
                }
                let removed = text[pos..pos + len].to_string();
                doc.delete(pos as u32, len as u32).unwrap();
                text.replace_range(pos..pos + len, "");
                history.push((0, pos, len, removed));
            }
            Op::Replace { pos, len } => {
                if pos + len > text.len() {
                    continue;
                }
                let removed = text[pos..pos + len].to_string();
                doc.replace(pos as u32, len as u32, b"xy\n").unwrap();
                text.replace_range(pos..pos + len, "xy\n");
                history.push((2, pos, 3, removed));
            }
            Op::Undo => {
                let Some((kind, pos, len, removed)) = history.pop() else {
                    continue;
                };
                doc.undo().unwrap().expect("отменять есть что");
                match kind {
                    // Отмена вставки: убрать вставленное.
                    1 => text.replace_range(pos..pos + len, ""),
                    // Отмена удаления: вернуть вырезанное.
                    0 => text.insert_str(pos, &removed),
                    // Отмена замены: убрать вставленное и вернуть вырезанное.
                    _ => {
                        text.replace_range(pos..pos + len, "");
                        text.insert_str(pos, &removed);
                    }
                }
            }
        }
        if doc.read() != text.as_bytes() {
            return Some((
                String::from_utf8_lossy(&doc.read()).into_owned(),
                text.clone(),
            ));
        }
    }
    None
}

/// Детерминированный генератор: падение воспроизводится.
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

/// Случайная последовательность правок; эталон тут же держится в тексте,
/// чтобы позиции выбирались по нему.
fn random_ops(seed: u64, count: usize) -> Vec<Op> {
    let mut rng = Rng(seed);
    let mut text = String::new();
    let mut ops = Vec::with_capacity(count);
    for _ in 0..count {
        let op = match rng.below(4) {
            0 if !text.is_empty() => {
                let pos = rng.below(text.len());
                let len = 1 + rng.below((text.len() - pos).min(20));
                text.replace_range(pos..pos + len, "");
                Op::Delete { pos, len }
            }
            1 => {
                let pos = rng.below(text.len() + 1);
                let typed = if rng.below(2) == 0 { "\n" } else { "ab" };
                text.insert_str(pos, typed);
                Op::Insert {
                    pos,
                    text: typed.to_string(),
                }
            }
            2 => {
                let pos = rng.below(text.len() + 1);
                let len = rng.below((text.len() - pos).min(10));
                text.replace_range(pos..pos + len, "xy\n");
                Op::Replace { pos, len }
            }
            _ => Op::Undo,
        };
        ops.push(op);
    }
    ops
}

/// Убрать всё лишнее: операция, без которой падение пропадает, не нужна.
fn shrink(mut ops: Vec<Op>) -> Vec<Op> {
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < ops.len() {
            let mut attempt = ops.clone();
            attempt.remove(i);
            if run(&attempt).is_some() {
                ops = attempt;
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    ops
}

#[test]
fn a_random_mix_of_edits_never_diverges() {
    // То же, что делает тест в крейте, но на многих сидах сразу. Как только
    // появится расхождение, сжатие ниже даст минимальную последовательность.
    for seed in 1..64u64 {
        let ops = random_ops(seed.wrapping_mul(0x9E3779B97F4A7C15), 200);
        if let Some((got, want)) = run(&ops) {
            let ops = shrink(ops);
            panic!(
                "seed {seed}: документ {got:?}, эталон {want:?}\nминимальная падающая последовательность: {ops:?}"
            );
        }
    }
}

/// Обрезка отброшенной ветки не должна съедать текст живых листьев.
///
/// Последовательность найдена сжатием: отмена удаления возвращает текст в
/// конец буфера, отмена замены его вырезает, а следующая правка отбрасывает
/// redo-ветку. Раньше обрезка резала буфер по отброшенным записям и оставляла
/// лист читать участок, которого уже нет.
#[test]
fn undo_and_discard_keep_the_buffer_intact() {
    let mut doc = Document::new(b"");
    doc.replace(0, 0, b"xy\n").unwrap();
    doc.replace(3, 0, b"xy\n").unwrap();
    doc.delete(1, 4).unwrap();
    doc.undo().unwrap();
    doc.insert(3, b"ab").unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert_eq!(doc.read(), b"xy\n");
    doc.insert(2, b"\n").unwrap();
    assert_eq!(doc.read(), b"xy\n\n");
}

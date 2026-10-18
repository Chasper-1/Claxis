//! Замеры rope.
//!
//! Те же самые замеры и тот же документ, что в `claxis-rope`: иначе цифры
//! нельзя сравнивать между крейтами — сравнивать можно только одинаковое.

use std::time::Instant;

use claxis_buffer::Document;

/// Число **вставок**, а не листьев: лист появляется там, где вставка режет
/// существующий диапазон, и их всегда больше, чем вставок.
const INSERTS: [u32; 3] = [1_000, 4_000, 8_000];
const ROUNDS: u32 = 20_000;

/// Тот же документ, что и у буфера: 500 КБ основания и рассеянные вставки.
fn document(n: u32) -> Document {
    let mut doc = Document::new(vec![b'a'; 500_000]);
    for i in 0..n {
        doc.insert((i * 37) % (doc.len() + 1), b"xx").unwrap();
    }
    doc
}

fn bench_insert() {
    println!("--- вставка и удаление ---");
    println!(
        "{:>9} | {:>8} | {:>7} | {:>10} | {:>10} | {:>8}",
        "вставок", "листьев", "высота", "середина", "конец", "снапш."
    );
    for target in INSERTS {
        let mut doc = document(target);
        let leaves = doc.leaves().len();
        // Высота — на документе, до замеров: циклы правок с широким удалением
        // снимают снапшот, и дерево схлопывается, а это не свойство rope.
        let height = doc.tree_debug_stats().expect("дерево не зациклено").height;
        let mid = doc.len() / 2;

        let snaps_before = doc.snapshots_taken();
        let start = Instant::now();
        for _ in 0..ROUNDS {
            doc.insert(mid, b"y").unwrap();
            doc.delete(mid, 1).unwrap();
        }
        let each = start.elapsed() / (ROUNDS * 2);
        let mid_snaps = doc.snapshots_taken() - snaps_before;

        let snaps_before = doc.snapshots_taken();
        let start = Instant::now();
        for _ in 0..ROUNDS {
            doc.insert(doc.len(), b"y").unwrap();
            doc.delete(doc.len() - 1, 1).unwrap();
        }
        let end = start.elapsed() / (ROUNDS * 2);
        let end_snaps = doc.snapshots_taken() - snaps_before;

        println!(
            "{:>9} | {:>8} | {:>7} | {:>10} | {:>10} | {:>8}",
            target,
            leaves,
            height,
            fmt(each),
            fmt(end),
            mid_snaps + end_snaps
        );
    }
}

fn bench_read() {
    println!("--- чтение ---");
    println!(
        "{:>9} | {:>8} | {:>9} | {:>10} | {:>10}",
        "вставок", "листьев", "байт", "всё целиком", "диапазон"
    );
    for target in INSERTS {
        let doc = document(target);
        let leaves = doc.leaves().len();
        let bytes = doc.len();

        let start = Instant::now();
        for _ in 0..50 {
            let text = doc.read();
            std::hint::black_box(&text);
        }
        let full = start.elapsed() / 50;

        let mid = doc.len() / 2;
        let part_start = Instant::now();
        for _ in 0..50 {
            let text = doc.read_range(mid, mid + 4096);
            std::hint::black_box(&text);
        }
        let part = part_start.elapsed() / 50;

        println!(
            "{:>9} | {:>8} | {:>9} | {:>10} | {:>10}",
            target,
            leaves,
            bytes,
            fmt(full),
            fmt(part)
        );
    }
}

/// Курсор ходит по документу туда и обратно: позиция меняется, правка следует
/// за ней. Это самый частый случай в редакторе.
fn bench_cursor() {
    println!("--- ходьба курсора (правка за позицией) ---");
    println!(
        "{:>9} | {:>8} | {:>10} | {:>8}",
        "вставок", "листьев", "на операцию", "снапш."
    );
    for target in INSERTS {
        let mut doc = document(target);
        let leaves = doc.leaves().len();
        let len = doc.len();
        let snaps_before = doc.snapshots_taken();
        let start = Instant::now();
        let mut pos = len / 2;
        for i in 0..ROUNDS {
            pos = if i % 2 == 0 {
                (pos + 7) % len
            } else {
                (pos + len - 7) % len
            };
            doc.insert(pos, b"y").unwrap();
            doc.delete(pos, 1).unwrap();
        }
        let each = start.elapsed() / (ROUNDS * 2);
        println!(
            "{:>9} | {:>8} | {:>10} | {:>8}",
            target,
            leaves,
            fmt(each),
            doc.snapshots_taken() - snaps_before
        );
    }
}

/// Широкое удаление с отменой: удаление половины документа идёт по дереву за
/// O(log n). Это самый дорогой замер.
fn bench_wide_delete() {
    println!("--- широкое удаление половины документа + отмена ---");
    println!(
        "{:>9} | {:>8} | {:>10} | {:>8}",
        "вставок", "листьев", "на операцию", "снапш."
    );
    for target in INSERTS {
        let mut doc = document(target);
        let leaves = doc.leaves().len();
        let len = doc.len();
        let snaps_before = doc.snapshots_taken();
        let start = Instant::now();
        for _ in 0..500 {
            doc.delete(0, len / 2).unwrap();
            doc.undo().unwrap().expect("отменяемо");
        }
        let each = start.elapsed() / 500;
        println!(
            "{:>9} | {:>8} | {:>10} | {:>8}",
            target,
            leaves,
            fmt(each),
            doc.snapshots_taken() - snaps_before
        );
    }
}

/// Память: сколько стоит документ после n правок.
fn bench_memory() {
    println!("--- память после правок ---");
    println!(
        "{:>9} | {:>8} | {:>9} | {:>9} | {:>9} | {:>10}",
        "вставок", "листьев", "узлы", "вершины", "текст", "всего"
    );
    for target in INSERTS {
        let doc = document(target);
        let m = doc.memory();
        println!(
            "{:>9} | {:>8} | {:>9} | {:>9} | {:>9} | {:>10}",
            target,
            m.leaves,
            m.nodes,
            m.versions,
            m.text_capacity,
            m.tree_bytes() + m.text_capacity + m.versions * m.version_size
        );
    }
}

fn fmt(d: std::time::Duration) -> String {
    let ns = d.as_nanos();
    if ns < 10_000 {
        format!("{ns}ns")
    } else if ns < 10_000_000 {
        format!("{:.1}µs", ns as f64 / 1000.0)
    } else {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    }
}

fn main() {
    println!(
        "глубина истории: {} записей (снапшот по исчерпании)",
        claxis_buffer::DEFAULT_DEPTH
    );
    println!("размер узла: {} байт", claxis_buffer::Node::size());
    println!();
    bench_insert();
    println!();
    bench_read();
    println!();
    bench_cursor();
    println!();
    bench_wide_delete();
    println!();
    bench_memory();
}

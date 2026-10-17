//! Замеры rope (новый крейт `claxis-rope`).
//!
//! Те же самые замеры и тот же документ, что в `claxis-buffer`: иначе цифры
//! нельзя сравнивать между крейтами — сравнивать можно только одинаковое.
//!
//! Новый крейт не имеет истории правок, снапшотов и отмены — это отдельная
//! задача. Поэтому замеры широкого удаления с отменой здесь отсутствуют:
//! сравнивать нечего.

use std::time::Instant;

use claxis_rope::{Document, chunk_size};

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
        "{:>9} | {:>8} | {:>10} | {:>10}",
        "вставок", "чанк", "середина", "конец"
    );
    for target in INSERTS {
        let mut doc = document(target);
        let chunk = doc.chunk_size();
        let mid = doc.len() / 2;

        let start = Instant::now();
        for _ in 0..ROUNDS {
            doc.insert(mid, b"y").unwrap();
            doc.delete(mid, 1).unwrap();
        }
        let each = start.elapsed() / (ROUNDS * 2);

        let start = Instant::now();
        for _ in 0..ROUNDS {
            doc.insert(doc.len(), b"y").unwrap();
            doc.delete(doc.len() - 1, 1).unwrap();
        }
        let end = start.elapsed() / (ROUNDS * 2);

        println!(
            "{:>9} | {:>8} | {:>10} | {:>10}",
            target,
            chunk,
            fmt(each),
            fmt(end)
        );
    }
}

fn bench_read() {
    println!("--- чтение ---");
    println!(
        "{:>9} | {:>8} | {:>9} | {:>10} | {:>10}",
        "вставок", "чанк", "байт", "всё целиком", "диапазон"
    );
    for target in INSERTS {
        let doc = document(target);
        let chunk = doc.chunk_size();
        let bytes = doc.len();

        let start = Instant::now();
        for _ in 0..50 {
            let text = doc.to_vec();
            std::hint::black_box(&text);
        }
        let full = start.elapsed() / 50;

        let mid = doc.len() / 2;
        let part_start = Instant::now();
        for _ in 0..50 {
            let text = doc.get(mid, 4096).unwrap();
            std::hint::black_box(text);
        }
        let part = part_start.elapsed() / 50;

        println!(
            "{:>9} | {:>8} | {:>9} | {:>10} | {:>10}",
            target,
            chunk,
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
    println!("{:>9} | {:>8} | {:>10}", "вставок", "чанк", "на операцию");
    for target in INSERTS {
        let mut doc = document(target);
        let chunk = doc.chunk_size();
        let len = doc.len();
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
        println!("{:>9} | {:>8} | {:>10}", target, chunk, fmt(each));
    }
}

/// Широкое удаление: удаление половины документа. В новом крейте нет отмены,
/// поэтому замеряет только само удаление.
fn bench_wide_delete() {
    println!("--- широкое удаление половины документа ---");
    println!("{:>9} | {:>8} | {:>10}", "вставок", "чанк", "на операцию");
    for target in INSERTS {
        let mut doc = document(target);
        let chunk = doc.chunk_size();
        let len = doc.len();
        let start = Instant::now();
        for _ in 0..500 {
            doc.delete(0, len / 2).unwrap();
            doc.insert(0, &vec![b'a'; len as usize / 2]).unwrap();
        }
        let each = start.elapsed() / 1000;
        println!("{:>9} | {:>8} | {:>10}", target, chunk, fmt(each));
    }
}

/// Память: сколько стоит документ после n правок.
fn bench_memory() {
    println!("--- память после правок ---");
    println!(
        "{:>9} | {:>8} | {:>9} | {:>9} | {:>10}",
        "вставок", "листьев", "узлы", "текст", "всего"
    );
    for target in INSERTS {
        let doc = document(target);
        let (leaves, nodes) = count_nodes(doc.tree().root());
        let text = doc.len() as usize;
        // Узел: Leaf(8 байт) или Branch(Vec 24 байта + u32 4 байта = 28 байт).
        let node_bytes = leaves * 8 + (nodes - leaves) * 28;
        let total = text + node_bytes;
        println!(
            "{:>9} | {:>8} | {:>9} | {:>9} | {:>10}",
            target, leaves, nodes, text, total
        );
    }
}

/// Считает листья и узлы в дереве.
fn count_nodes(node: &claxis_rope::Node) -> (usize, usize) {
    use claxis_rope::Node;
    match node {
        Node::Leaf(_) => (1, 1),
        Node::Branch(branch) => {
            let mut leaves = 0;
            let mut nodes = 1;
            for child in branch.children() {
                let (l, n) = count_nodes(child);
                leaves += l;
                nodes += n;
            }
            (leaves, nodes)
        }
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
    println!("чанк для 500 КБ: {} байт", chunk_size(500_000));
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

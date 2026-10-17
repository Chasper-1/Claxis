//! Замеры rope.

use std::time::Instant;

use claxis_rope::{Document, chunk_size};

/// Замер: сколько времени занимает одна операция.
fn measure(name: &str, bytes: u32, round: impl FnOnce(&mut Document)) {
    let mut doc = Document::new(vec![b'a'; bytes as usize]);
    let start = Instant::now();
    round(&mut doc);
    let spent = start.elapsed();
    println!(
        "{name}: {} байт, чанк {}, {:?}, документ {} байт",
        bytes,
        chunk_size(bytes as usize),
        spent,
        doc.len()
    );
}

fn main() {
    measure("вставка в середину", 1024 * 1024, |doc| {
        for _ in 0..1000 {
            let at = doc.len() / 2;
            doc.insert(at, "абв".as_bytes()).unwrap();
        }
    });
    measure("вырезание с начала", 1024 * 1024, |doc| {
        for _ in 0..1000 {
            doc.delete(0, 2).unwrap();
        }
    });
    measure("вставка в конец", 4 * 1024 * 1024, |doc| {
        for _ in 0..1000 {
            doc.insert(doc.len(), "абв".as_bytes()).unwrap();
        }
    });
}

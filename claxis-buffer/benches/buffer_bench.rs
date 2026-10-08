use std::time::Instant;

use claxis_buffer::Buffer;

const HISTORY: u32 = 40_000;
const SEGMENTS: usize = 28_000;
const ITERS: usize = 20;

/// Документ с `n` сегментами, полученный рассеянными вставками в начало.
/// Каждая операция меряется сама по себе, на своём размере.
fn scattered(n: u32) -> Buffer {
    let mut b = Buffer::new(&vec![b'a'; 200_000]);
    for i in 0..n {
        let pos = (i * 37) % (b.len() + 1);
        b.insert(pos, b"xx").unwrap();
    }
    b
}

fn bench_insert_at_end() {
    let mut b = Buffer::new("");
    for _ in 0..HISTORY {
        b.insert(b.len(), b"z").unwrap();
    }
    let segs = b.segments().len();
    let start = Instant::now();
    for _ in 0..ITERS {
        b.insert(b.len(), b"z").unwrap();
        b.delete(b.len() - 1, 1).unwrap();
    }
    println!(
        "insert_at_end: {:?}  ({segs} сегментов)",
        start.elapsed() / (ITERS * 2) as u32
    );
}

fn bench_insert_middle() {
    for n in [1_000u32, 10_000, 20_000] {
        let mut b = scattered(n);
        let segs = b.segments().len();
        let start = Instant::now();
        let rounds = 1_000;
        for _ in 0..rounds {
            b.insert(b.len() / 2, b"y").unwrap();
            b.delete(b.len() / 2, 1).unwrap();
        }
        println!(
            "insert_middle: {:?}  ({segs} сегментов)",
            start.elapsed() / (rounds * 2) as u32
        );
    }
}

fn bench_read() {
    let original = vec![b'a'; 1_000_000];
    let mut b = Buffer::new(&original);
    for i in 0..SEGMENTS as u32 {
        let pos = (i * 37) % (b.len() + 1);
        b.insert(pos, b"[]").unwrap();
    }
    let segments = b.segments().len();
    let start = Instant::now();
    for _ in 0..ITERS {
        let data = b.read();
        std::hint::black_box(&data);
    }
    println!(
        "read: {:?}  ({segments} сегментов, {} байт)",
        start.elapsed() / ITERS as u32,
        b.len()
    );
}

fn bench_rebuild() {
    let mut b = Buffer::new("");
    for i in 0..HISTORY {
        b.insert(i % (b.len() + 1), b"x").unwrap();
    }
    let segs = b.segments().len();
    let start = Instant::now();
    for _ in 0..ITERS {
        b.rebuild();
    }
    println!(
        "rebuild: {:?}  ({segs} сегментов, {HISTORY} правок)",
        start.elapsed() / ITERS as u32
    );
    assert_eq!(b.len(), HISTORY);
}

fn main() {
    bench_insert_at_end();
    bench_insert_middle();
    bench_read();
    bench_rebuild();
}

use std::time::Instant;

use claxis_buffer::Buffer;

const HISTORY: usize = 40_000;
const SEGMENTS: usize = 28_000;
const ITERS: usize = 20;

fn rebuild_bench() {
    let mut b = Buffer::new("");
    for i in 0..HISTORY {
        b.insert(i % (b.len() + 1), b"x").unwrap();
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        b.rebuild();
    }
    let avg = start.elapsed() / ITERS as u32;
    println!("rebuild: {avg:?}  ({HISTORY} правок в истории)");
    assert_eq!(b.len(), HISTORY);
}

fn read_bench() {
    let original = vec![b'a'; 1_000_000];
    let mut b = Buffer::new(&original);
    for i in 0..SEGMENTS {
        let pos = (i * 37) % (b.len() + 1);
        b.insert(pos, b"[]").unwrap();
    }
    let segments = b.segments().count();
    let start = Instant::now();
    for _ in 0..ITERS {
        let data = b.read();
        std::hint::black_box(&data);
    }
    let avg = start.elapsed() / ITERS as u32;
    println!("read: {avg:?}  ({segments} сегментов, {} байт)", b.len());
}

fn insert_bench() {
    let mut b = Buffer::new("");
    let start = Instant::now();
    for i in 0..HISTORY {
        b.insert(i, b"x").unwrap();
    }
    let avg = start.elapsed() / HISTORY as u32;
    println!("insert: {avg:?}  ({HISTORY} вставок в конец)");
}

fn main() {
    rebuild_bench();
    read_bench();
    insert_bench();
}

use std::time::Instant;

use claxis_buffer::Buffer;

const SEGMENTS: [u32; 3] = [1_000, 8_000, 16_000];
const ROUNDS: u32 = 20_000;

/// Документ из `n` сегментов: рассеянные вставки, каждая разрезает исходник.
fn document(n: u32) -> Buffer {
    let mut b = Buffer::new(vec![b'a'; 500_000]);
    for i in 0..n {
        b.insert((i * 37) % (b.len() + 1), b"xx").unwrap();
    }
    b
}

fn bench_insert() {
    for target in SEGMENTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let mid = b.len() / 2;
        let start = Instant::now();
        for _ in 0..ROUNDS {
            b.insert(mid, b"y").unwrap();
            b.delete(mid, 1).unwrap();
        }
        let each = start.elapsed() / (ROUNDS * 2);
        let start = Instant::now();
        for _ in 0..ROUNDS {
            b.insert(b.len(), b"y").unwrap();
            b.delete(b.len() - 1, 1).unwrap();
        }
        let end = start.elapsed() / (ROUNDS * 2);
        println!("insert: середина {each:?}, конец {end:?}  ({segs} сегментов)");
    }
}

fn bench_read() {
    for target in SEGMENTS {
        let b = document(target);
        let segs = b.segments().len();
        let bytes = b.len();
        let start = Instant::now();
        for _ in 0..50 {
            let text = b.read();
            std::hint::black_box(&text);
        }
        println!(
            "read: {:?}  ({segs} сегментов, {bytes} байт)",
            start.elapsed() / 50
        );
    }
}

fn bench_cursor() {
    for target in SEGMENTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let len = b.len();
        let start = Instant::now();
        let mut pos = len / 2;
        for i in 0..ROUNDS {
            pos = if i % 2 == 0 {
                (pos + 7) % len
            } else {
                (pos + len - 7) % len
            };
            b.insert(pos, b"y").unwrap();
            b.delete(pos, 1).unwrap();
        }
        println!(
            "cursor: {:?}  ({segs} сегментов, шаг 7 байт)",
            start.elapsed() / (ROUNDS * 2)
        );
    }
}

fn bench_wide_delete() {
    for target in SEGMENTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let len = b.len();
        let start = Instant::now();
        for _ in 0..2_000 {
            b.delete(0, len / 2).unwrap();
            b.undo().unwrap();
        }
        println!(
            "wide_delete: {:?}  ({segs} сегментов, половина документа)",
            start.elapsed() / 2_000_u32
        );
    }
}

fn main() {
    bench_insert();
    bench_read();
    bench_cursor();
    bench_wide_delete();
}

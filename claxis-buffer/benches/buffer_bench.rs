use std::time::Instant;

use claxis_buffer::Buffer;

/// Число **вставок**, а не сегментов: снапшот держит дерево в пределах
/// `MAX_LEAVES`, поэтому сегментов в документе всегда меньше, чем вставок.
const INSERTS: [u32; 3] = [1_000, 4_000, 8_000];
const ROUNDS: u32 = 20_000;

/// Документ из `n` рассеянных вставок. Каждая разрезает существующий сегмент,
/// поэтому сегментов получается примерно `2n + 1` — если не сработал снапшот.
fn document(n: u32) -> Buffer {
    let mut b = Buffer::new(vec![b'a'; 500_000]);
    for i in 0..n {
        b.insert((i * 37) % (b.len() + 1), b"xx").unwrap();
    }
    b
}

/// Сколько раз брался снапшот за замер — иначе непонятно, что именно измерено.
fn snaps(b: &Buffer) -> u64 {
    b.snapshots_taken()
}

fn bench_insert() {
    println!("--- вставка и удаление ---");
    println!(
        "{:>9} | {:>8} | {:>7} | {:>10} | {:>10} | {:>8}",
        "вставок", "сегментов", "высота", "середина", "конец", "снапш."
    );
    for target in INSERTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let mid = b.len() / 2;

        let snaps_before = snaps(&b);
        let start = Instant::now();
        for _ in 0..ROUNDS {
            b.insert(mid, b"y").unwrap();
            b.delete(mid, 1).unwrap();
        }
        let each = start.elapsed() / (ROUNDS * 2);
        let mid_snaps = snaps(&b) - snaps_before;

        let snaps_before = snaps(&b);
        let start = Instant::now();
        for _ in 0..ROUNDS {
            b.insert(b.len(), b"y").unwrap();
            b.delete(b.len() - 1, 1).unwrap();
        }
        let end = start.elapsed() / (ROUNDS * 2);
        let end_snaps = snaps(&b) - snaps_before;

        println!(
            "{:>9} | {:>8} | {:>7} | {:>10} | {:>10} | {:>8}",
            target,
            segs,
            b.tree_height(),
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
        "вставок", "сегментов", "байт", "всё целиком", "диапазон"
    );
    for target in INSERTS {
        let b = document(target);
        let segs = b.segments().len();
        let bytes = b.len();

        let start = Instant::now();
        for _ in 0..50 {
            let text = b.read();
            std::hint::black_box(&text);
        }
        let full = start.elapsed() / 50;

        // Частичное чтение — то, что редактор делает постоянно.
        let mid = b.len() / 2;
        let part_start = Instant::now();
        for _ in 0..50 {
            let text = b.read_range(mid, mid + 4096);
            std::hint::black_box(&text);
        }
        let part = part_start.elapsed() / 50;

        println!(
            "{:>9} | {:>8} | {:>9} | {:>10} | {:>10}",
            target,
            segs,
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
        "вставок", "сегментов", "на операцию", "снапш."
    );
    for target in INSERTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let len = b.len();
        let snaps_before = snaps(&b);
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
        let each = start.elapsed() / (ROUNDS * 2);
        println!(
            "{:>9} | {:>8} | {:>10} | {:>8}",
            target,
            segs,
            fmt(each),
            snaps(&b) - snaps_before
        );
    }
}

/// Широкое удаление с отменой: удаление половины документа идёт по дереву за
/// O(log n), отмена пересобирает дерево целиком. Это самый дорогой замер.
fn bench_wide_delete() {
    println!("--- широкое удаление половины документа + отмена ---");
    println!(
        "{:>9} | {:>8} | {:>10} | {:>8}",
        "вставок", "сегментов", "на операцию", "снапш."
    );
    for target in INSERTS {
        let mut b = document(target);
        let segs = b.segments().len();
        let len = b.len();
        let snaps_before = snaps(&b);
        let start = Instant::now();
        for _ in 0..500 {
            b.delete(0, len / 2).unwrap();
            b.undo().unwrap();
        }
        let each = start.elapsed() / 500;
        println!(
            "{:>9} | {:>8} | {:>10} | {:>8}",
            target,
            segs,
            fmt(each),
            snaps(&b) - snaps_before
        );
    }
}

/// Отдельный замер самого снапшота: он снимает весь текст и строит дерево заново.
fn bench_snapshot() {
    println!("--- снятие снапшота ---");
    let mut b = document(4_000);
    let segs = b.segments().len();
    let start = Instant::now();
    for _ in 0..50 {
        b.snapshot().unwrap();
    }
    let each = start.elapsed() / 50;
    println!(
        "снапшот: {}  ({segs} сегментов, документ {} байт)",
        fmt(each),
        b.len()
    );
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
    println!(
        "размер узла: {} байт, сегмента: {} байт",
        Buffer::node_size(),
        Buffer::segment_size()
    );
    println!();
    bench_insert();
    println!();
    bench_read();
    println!();
    bench_cursor();
    println!();
    bench_wide_delete();
    println!();
    bench_snapshot();
}

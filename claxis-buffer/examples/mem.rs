use std::mem::size_of;

use claxis_buffer::{ArenaSize, Buffer};

/// Текущий Resident Set Size из `/proc/self/status`, в килобайтах.
fn rss_kb() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
            return digits.parse().unwrap_or(0);
        }
    }
    0
}

/// Помечает границу, чтобы аллокатор не отдал память обратно ОС.
fn keep<T>(v: T) -> Box<T> {
    Box::new(v)
}

fn main() {
    println!("=== размеры структур ===");
    println!(
        "Record   {} байт   Segment {} байт   Node {} байт",
        size_of::<claxis_buffer::Record>(),
        Buffer::segment_size(),
        Buffer::node_size()
    );
    println!();

    println!("=== исходный текст 1 МБ, вставки в конец ===");
    println!(
        "{:>9} | {:>7} | {:>7} | {:>10} | {:>10} | {:>12}",
        "вставок", "листьев", "высота", "дерево", "Added", "на сегмент"
    );
    for &n in &[0usize, 1_000, 10_000, 100_000, 500_000] {
        let base = rss_kb();
        let buf = keep(build(n));
        let m = buf.tree_memory();
        let rss = rss_kb().saturating_sub(base) * 1024;
        println!(
            "{:>9} | {:>7} | {:>7} | {:>10} | {:>10} | {:>12}",
            n,
            m.leaves,
            buf.tree_height(),
            fmt_bytes(m.tree_bytes()),
            fmt_bytes(m.added_capacity),
            fmt_bytes(m.tree_bytes() / m.leaves.max(1))
        );
        let _ = rss;
    }
    println!();

    println!("=== вставки в середину (тот же документ, другой рисунок) ===");
    println!(
        "{:>9} | {:>7} | {:>7} | {:>10}",
        "вставок", "листьев", "высота", "дерево"
    );
    for &n in &[1_000usize, 10_000, 100_000] {
        let buf = keep(build_mid(n));
        let m = buf.tree_memory();
        println!(
            "{:>9} | {:>7} | {:>7} | {:>10}",
            n,
            m.leaves,
            buf.tree_height(),
            fmt_bytes(m.tree_bytes())
        );
    }
    println!();

    println!("=== реальный RSS процесса ===");
    let base = rss_kb();
    let buf = keep(build(200_000));
    let after = rss_kb();
    println!("документ 200K вставок + 1 МБ оригинала:");
    println!("  RSS до      {base} КБ");
    println!("  RSS после   {after} КБ");
    println!("  прирост     {} КБ", after.saturating_sub(base));
    let m = buf.tree_memory();
    println!("  {m}");
    println!(
        "  байт на сегмент: {:.1}",
        m.tree_bytes() as f64 / m.leaves.max(1) as f64
    );
    println!(
        "  Added на символ: {:.3}",
        m.added_capacity as f64 / buf.len().max(1) as f64
    );
    println!();

    println!("=== реалистичная работа: вставка порциями ===");
    println!(
        "{:>10} | {:>7} | {:>7} | {:>10} | {:>12} | {:>10}",
        "порций", "листьев", "высота", "дерево", "Added", "Б/сегмент"
    );
    for &chunk in &[1usize, 8, 20, 80, 400] {
        for &total in &[100_000usize] {
            let mut buf = Buffer::new(vec![b'a'; 1_000_000]);
            let piece = vec![b'x'; chunk];
            let mut pos = buf.len() / 2;
            let mut done = 0;
            while done < total {
                buf.insert(pos, &piece).unwrap();
                // курсор гуляет по документу
                pos = (pos + 37) % buf.len();
                done += chunk;
            }
            let m = buf.tree_memory();
            println!(
                "{:>10} | {:>7} | {:>7} | {:>10} | {:>12} | {:>10.1}",
                total,
                m.leaves,
                buf.tree_height(),
                fmt_bytes(m.tree_bytes()),
                fmt_bytes(m.added_capacity),
                m.tree_bytes() as f64 / m.leaves.max(1) as f64
            );
        }
    }
    println!();

    println!("=== арена записей ===");
    for size in [
        ArenaSize::Kb32,
        ArenaSize::Kb128,
        ArenaSize::Kb512,
        ArenaSize::Kb1024,
    ] {
        println!(
            "{:?}: блок {} записей ({} байт)",
            size,
            size.records(),
            size.bytes()
        );
    }
    println!("{}", build(1000).tree_memory());
}

fn build(n: usize) -> Buffer {
    let mut buf = Buffer::new(vec![b'a'; 1_000_000]);
    for _ in 0..n {
        buf.insert(buf.len(), b"x").unwrap();
    }
    buf
}

fn build_mid(n: usize) -> Buffer {
    let mut buf = Buffer::new(vec![b'a'; 1_000_000]);
    let mid = buf.len() / 2;
    for i in 0..n {
        buf.insert(mid + (i % 64) as u32, b"x").unwrap();
    }
    buf
}

fn fmt_bytes(b: usize) -> String {
    if b >= 1024 * 1024 {
        format!("{:.2} МБ", b as f64 / 1048576.0)
    } else if b >= 1024 {
        format!("{:.1} КБ", b as f64 / 1024.0)
    } else {
        format!("{b} Б")
    }
}

use claxis_buffer::Buffer;

fn main() {
    println!(
        "{:>9} | {:>8} | {:>9} | {:>10} | {:>4}",
        "глубина", "предел", "узлов", "дерево", "пик:"
    );
    for depth in [1u32, 10, 100, 500, 1_000, 4_000, 8_192] {
        // Рассеянные вставки: каждая разрезает сегмент, листья растут.
        // Снапшот сбрасывает дерево, поэтому важно пиковое число листьев,
        // а не итоговое.
        let mut buf = Buffer::with_history_depth(b"", depth).unwrap();
        let want = 2 * depth as usize + 1;
        let mut peak = 0usize;
        for i in 0..(depth * 12) {
            let pos = (i * 37) % (buf.len() + 1);
            buf.insert(pos, b"x").unwrap();
            peak = peak.max(buf.segments().len());
        }
        let m = buf.tree_memory();
        assert!(
            peak <= want,
            "глубина {depth}: пик листьев {peak} больше предела {want}"
        );
        println!(
            "{:>9} | {:>8} | {:>9} | {:>10} | {:>4}/{:>4}",
            depth,
            want,
            m.pool_capacity,
            human(m.tree_bytes()),
            peak,
            want
        );
    }
    println!();
    let small = Buffer::with_history_depth(b"", 100).unwrap();
    let m = small.tree_memory();
    println!(
        "пустой документ при глубине 100: {} узлов, {}.",
        m.pool_capacity,
        human(m.tree_bytes())
    );
    println!("Пул считается от глубины, а не от текущего документа: выделяется сразу,");
    println!("потом расти нельзя — иначе в горячем пути появится переаллокация.");
}

fn human(b: usize) -> String {
    if b >= 1024 * 1024 {
        format!("{:.2} МБ", b as f64 / 1048576.0)
    } else if b >= 1024 {
        format!("{:.1} КБ", b as f64 / 1024.0)
    } else {
        format!("{b} Б")
    }
}

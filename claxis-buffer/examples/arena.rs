use claxis_buffer::{Buffer, DEFAULT_DEPTH, MAX_DEPTH};

fn main() {
    println!("глубина по умолчанию: {DEFAULT_DEPTH}");
    println!("предел глубины: {MAX_DEPTH}");
    println!();

    // Реалистичная сессия: печатаем, правим, отменяем.
    let mut buf = Buffer::new(b"");
    let mut pos = 0u32;
    for i in 0..20_000u32 {
        pos = (pos + 7) % (buf.len() + 1);
        buf.insert(pos, "слово ".as_bytes()).unwrap();
        if i % 5 == 0 && buf.len() > 10 {
            buf.delete(pos, 4).unwrap();
        }
    }
    let m = buf.tree_memory();
    println!("после 20k правок:");
    println!("  записей в арене: {}", buf.undo_stack().len());
    println!("  блоков арены: {}", m.arena_blocks);
    println!("  ёмкость блока: {} записей", m.arena_capacity);
    println!("  снапшотов: {}", buf.snapshots_taken());
    println!("  листьев: {} (потолок {})", m.leaves, 2 * 8_192 + 1);
    println!("  дерево: {} байт", m.tree_bytes());
    println!("  Added: {} из {} байт", m.added_len, m.added_capacity);
    println!("  документ: {} байт", buf.len());
    println!();

    // Проверяем, что история жива и отмена работает.
    let before = buf.len();
    buf.undo().unwrap().unwrap();
    println!("undo: {} -> {}", before, buf.len());
    buf.redo().unwrap().unwrap();
    println!("redo: -> {}", buf.len());
    println!();

    // Граница блока: переходим через неё и смотрим, что записи не теряются.
    let mut b2 = Buffer::with_history_depth(b"", 8_192).unwrap();
    for i in 0..20_000u32 {
        b2.insert(i, b"x").unwrap();
    }
    let m2 = b2.tree_memory();
    println!("арена через границу блока:");
    println!("  записей: {}", b2.undo_stack().len());
    println!("  блоков: {}", m2.arena_blocks);
    println!("  снапшотов: {}", b2.snapshots_taken());
    println!("  идентификаторы уникальны: да, если записей == уникальных id");
    let mut ids: Vec<u32> = b2.undo_stack().to_vec();
    ids.sort_unstable();
    ids.dedup();
    println!(
        "  уникальных id: {} из {}",
        ids.len(),
        b2.undo_stack().len()
    );
}

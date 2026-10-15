use std::path::{Path, PathBuf};

use crate::{SnapshotStore, StorePaths};

/// Отдельный временный каталог под каждый тест: базы не делятся.
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("claxis-store-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        Self { dir }
    }

    fn paths(&self) -> StorePaths {
        StorePaths::new(self.dir.clone())
    }

    fn store(&self, keep: u8) -> SnapshotStore {
        SnapshotStore::open(&self.paths(), keep).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn file(name: &str) -> PathBuf {
    Path::new("/temp/project").join(name)
}

#[test]
fn database_is_created_on_first_open() {
    let temp = Temp::new("create");
    let paths = temp.paths();
    assert!(!paths.exists());
    let _ = SnapshotStore::open(&paths, 3).unwrap();
    assert!(paths.exists(), "каталог должен быть создан");
    assert!(paths.database().is_file(), "файл базы должен появиться");
}

#[test]
fn snapshot_survives_reopen() {
    let temp = Temp::new("reopen");
    let target = file("a.rs");
    {
        let mut store = temp.store(3);
        store.put(&target, "первый".as_bytes(), b"").unwrap();
    }
    {
        let store = temp.store(3);
        let snapshot = store
            .latest(&target)
            .unwrap()
            .expect("снапшот должен сохраниться");
        assert_eq!(snapshot.text, "первый".as_bytes());
    }
}

#[test]
fn keep_limits_snapshots_per_file() {
    // Три снапшота на файл: четвёртый вытесняет самый старый.
    let temp = Temp::new("ring");
    let target = file("a.rs");
    let mut store = temp.store(3);

    for i in 0..5 {
        store
            .put(&target, format!("снапшот {i}").as_bytes(), b"")
            .unwrap();
    }

    assert_eq!(store.count(&target).unwrap(), 3);
    let list = store.list(&target).unwrap();
    let texts: Vec<String> = list
        .iter()
        .map(|s| String::from_utf8_lossy(&s.text).into_owned())
        .collect();
    assert_eq!(
        texts,
        vec!["снапшот 4", "снапшот 3", "снапшот 2"],
        "остались три свежих, свежие первыми"
    );
}

#[test]
fn zero_keep_saves_nothing_and_destroys_nothing() {
    // Ноль — честный выбор: на диск не пишется ничего, но вызовы не падают.
    let temp = Temp::new("keep-zero");
    let target = file("a.rs");
    let mut store = temp.store(0);
    for i in 0..5 {
        store.put(&target, format!("{i}").as_bytes(), b"").unwrap();
    }
    assert_eq!(
        store.count(&target).unwrap(),
        0,
        "при keep = 0 писать нечего"
    );
    assert!(store.latest(&target).unwrap().is_none());
}

#[test]
fn keep_is_respected_for_any_value() {
    // Настройка из конфига: хоть один снапшот, хоть десять.
    for keep in [1u8, 2, 7, 10] {
        let temp = Temp::new(&format!("keep{keep}"));
        let target = file("a.rs");
        let mut store = temp.store(keep);
        for i in 0..keep + 3 {
            store.put(&target, format!("{i}").as_bytes(), b"").unwrap();
        }
        assert_eq!(
            store.count(&target).unwrap(),
            keep as u32,
            "keep={keep} не соблюдён"
        );
    }
}

#[test]
fn different_files_have_separate_rings() {
    // Кольцо у каждого файла своё: снапшоты других файлов не вытесняются.
    let temp = Temp::new("separate");
    let a = file("a.rs");
    let b = file("b.rs");
    let mut store = temp.store(2);

    store.put(&a, b"a0", b"").unwrap();
    store.put(&a, b"a1", b"").unwrap();
    store.put(&a, b"a2", b"").unwrap();
    store.put(&b, b"b0", b"").unwrap();

    assert_eq!(store.count(&a).unwrap(), 2);
    assert_eq!(store.count(&b).unwrap(), 1);
    assert_eq!(store.total().unwrap(), 3);
    assert_eq!(store.latest(&b).unwrap().unwrap().text, b"b0");
}

#[test]
fn empty_text_is_stored_fine() {
    // Пустой документ — обычный снапшот.
    let temp = Temp::new("empty");
    let target = file("empty.rs");
    let mut store = temp.store(3);
    store.put(&target, b"", b"").unwrap();
    let snapshot = store.latest(&target).unwrap().unwrap();
    assert!(snapshot.text.is_empty());
}

#[test]
fn large_text_round_trips_unchanged() {
    // Много байт с любыми значениями, включая ноль и не-UTF8.
    let temp = Temp::new("large");
    let target = file("big.rs");
    let mut store = temp.store(3);
    let text: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
    store.put(&target, &text, b"").unwrap();
    let snapshot = store.latest(&target).unwrap().unwrap();
    assert_eq!(snapshot.text.len(), text.len(), "длина не совпала");
    assert_eq!(snapshot.text, text, "содержимое не совпало");
    // Нули в данных тоже должны выжить — это проверка на то, что блоб
    // сохраняется целиком, а не до первого нуля.
    let zeros = (0..500_000u32).filter(|i| i % 251 == 0).count();
    assert_eq!(snapshot.text.iter().filter(|&&b| b == 0).count(), zeros);
}

#[test]
fn get_by_seq_and_missing_is_none() {
    let temp = Temp::new("byseq");
    let target = file("a.rs");
    let mut store = temp.store(3);
    store.put(&target, "первый".as_bytes(), b"").unwrap();
    let seq = store.latest(&target).unwrap().unwrap().seq;
    let by_seq = store.get(&target, seq).unwrap().expect("снапшот по номеру");
    assert_eq!(by_seq.text, "первый".as_bytes());
    assert!(store.get(&target, 999_999).unwrap().is_none());
    assert!(store.latest(&file("нет-такого.rs")).unwrap().is_none());
}

#[test]
fn forget_removes_only_that_file() {
    let temp = Temp::new("forget");
    let a = file("a.rs");
    let b = file("b.rs");
    let mut store = temp.store(3);
    store.put(&a, b"a", b"").unwrap();
    store.put(&b, b"b", b"").unwrap();
    store.forget(&a).unwrap();
    assert_eq!(store.count(&a).unwrap(), 0);
    assert_eq!(store.count(&b).unwrap(), 1);
}

#[test]
fn reopening_keeps_existing_snapshots() {
    // Второе открытие не стирает то, что записано ранее.
    let temp = Temp::new("keep-on-reopen");
    let target = file("a.rs");
    {
        let mut store = temp.store(3);
        store
            .put(&target, "из первого запуска".as_bytes(), b"")
            .unwrap();
    }
    {
        let mut store = temp.store(3);
        assert_eq!(store.count(&target).unwrap(), 1);
        store
            .put(&target, "из второго запуска".as_bytes(), b"")
            .unwrap();
        let list = store.list(&target).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "из второго запуска".as_bytes());
        assert_eq!(list[1].text, "из первого запуска".as_bytes());
    }
}

#[test]
fn two_stores_on_one_database() {
    // Два независимых подключения к одной базе работают одновременно —
    // это то, ради чего выбрана sqlite, а не база с эксклюзивным локом.
    let temp = Temp::new("two");
    let target = file("a.rs");
    let mut first = temp.store(3);
    first.put(&target, "первый".as_bytes(), b"").unwrap();

    let mut second = SnapshotStore::open(&temp.paths(), 3).unwrap();
    second.put(&target, "второй".as_bytes(), b"").unwrap();

    assert_eq!(
        first.count(&target).unwrap(),
        2,
        "первый видит запись второго"
    );
    assert_eq!(
        second.count(&target).unwrap(),
        2,
        "второй видит запись первого"
    );
}

#[test]
fn size_on_disk_is_reported() {
    let temp = Temp::new("size");
    let target = file("a.rs");
    let mut store = temp.store(3);
    store.put(&target, &vec![b'x'; 100_000], b"").unwrap();
    let size = store.size_on_disk().unwrap();
    assert!(size > 0, "размер должен быть известен");
    assert!(store.keep() == 3);
}

#[test]
fn store_paths_from_env_have_claxis_dir() {
    // Путь по умолчанию заканчивается именем редактора.
    let paths = StorePaths::from_env();
    let dir = paths.dir.display().to_string();
    assert!(
        dir.ends_with("claxis"),
        "путь должен кончаться на claxis: {dir}"
    );
    assert!(paths.database().display().to_string().ends_with("store.db"));
}

#[test]
fn last_good_config_round_trip() {
    // Последний корректный конфиг переживает перезапуск редактора.
    let temp = Temp::new("last-good");
    let store = temp.store(3);
    assert!(store.last_good("config").unwrap().is_none());

    let mut store = store;
    store
        .save_last_good("config", "hold = { alt = 200, ctrl = 200 }")
        .unwrap();
    assert_eq!(
        store.last_good("config").unwrap().as_deref(),
        Some("hold = { alt = 200, ctrl = 200 }")
    );
}

#[test]
fn last_good_config_overwrites_and_forgets() {
    let temp = Temp::new("last-good-overwrite");
    let mut store = temp.store(3);

    store.save_last_good("config", "old").unwrap();
    store.save_last_good("config", "new").unwrap();
    assert_eq!(store.last_good("config").unwrap().as_deref(), Some("new"));

    // Другие имена не трогаются: конфиг и тема хранятся раздельно.
    store.save_last_good("edit", "edit body").unwrap();
    store.forget_last_good("config").unwrap();
    assert!(store.last_good("config").unwrap().is_none());
    assert_eq!(
        store.last_good("edit").unwrap().as_deref(),
        Some("edit body")
    );
}

#[test]
fn last_good_survives_reopening() {
    // Перезапуск редактора не должен терять сохранённый конфиг.
    let temp = Temp::new("last-good-reopen");
    {
        let mut store = temp.store(3);
        store.save_last_good("config", "body").unwrap();
    }
    let store = temp.store(3);
    assert_eq!(store.last_good("config").unwrap().as_deref(), Some("body"));
}

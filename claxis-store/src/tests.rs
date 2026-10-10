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
    Path::new("/tmp/project").join(name)
}

#[test]
fn database_is_created_on_first_open() {
    let tmp = Temp::new("create");
    let paths = tmp.paths();
    assert!(!paths.exists());
    let _ = SnapshotStore::open(&paths, 3).unwrap();
    assert!(paths.exists(), "каталог должен быть создан");
    assert!(paths.database().is_file(), "файл базы должен появиться");
}

#[test]
fn snapshot_survives_reopen() {
    let tmp = Temp::new("reopen");
    let f = file("a.rs");
    {
        let mut store = tmp.store(3);
        store.put(&f, "первый".as_bytes()).unwrap();
    }
    {
        let store = tmp.store(3);
        let snap = store
            .latest(&f)
            .unwrap()
            .expect("снапшот должен сохраниться");
        assert_eq!(snap.text, "первый".as_bytes());
    }
}

#[test]
fn keep_limits_snapshots_per_file() {
    // Три снапшота на файл: четвёртый вытесняет самый старый.
    let tmp = Temp::new("ring");
    let f = file("a.rs");
    let mut store = tmp.store(3);

    for i in 0..5 {
        store.put(&f, format!("снапшот {i}").as_bytes()).unwrap();
    }

    assert_eq!(store.count(&f).unwrap(), 3);
    let list = store.list(&f).unwrap();
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
    let tmp = Temp::new("keep-zero");
    let f = file("a.rs");
    let mut store = tmp.store(0);
    for i in 0..5 {
        store.put(&f, format!("{i}").as_bytes()).unwrap();
    }
    assert_eq!(store.count(&f).unwrap(), 0, "при keep = 0 писать нечего");
    assert!(store.latest(&f).unwrap().is_none());
}

#[test]
fn keep_is_respected_for_any_value() {
    // Настройка из конфига: хоть один снапшот, хоть десять.
    for keep in [1u8, 2, 7, 10] {
        let tmp = Temp::new(&format!("keep{keep}"));
        let f = file("a.rs");
        let mut store = tmp.store(keep);
        for i in 0..keep + 3 {
            store.put(&f, format!("{i}").as_bytes()).unwrap();
        }
        assert_eq!(
            store.count(&f).unwrap(),
            keep as u32,
            "keep={keep} не соблюдён"
        );
    }
}

#[test]
fn different_files_have_separate_rings() {
    // Кольцо у каждого файла своё: снапшоты других файлов не вытесняются.
    let tmp = Temp::new("separate");
    let a = file("a.rs");
    let b = file("b.rs");
    let mut store = tmp.store(2);

    store.put(&a, b"a0").unwrap();
    store.put(&a, b"a1").unwrap();
    store.put(&a, b"a2").unwrap();
    store.put(&b, b"b0").unwrap();

    assert_eq!(store.count(&a).unwrap(), 2);
    assert_eq!(store.count(&b).unwrap(), 1);
    assert_eq!(store.total().unwrap(), 3);
    assert_eq!(store.latest(&b).unwrap().unwrap().text, b"b0");
}

#[test]
fn empty_text_is_stored_fine() {
    // Пустой документ — обычный снапшот.
    let tmp = Temp::new("empty");
    let f = file("empty.rs");
    let mut store = tmp.store(3);
    store.put(&f, b"").unwrap();
    let snap = store.latest(&f).unwrap().unwrap();
    assert!(snap.text.is_empty());
}

#[test]
fn large_text_round_trips_unchanged() {
    // Много байт с любыми значениями, включая ноль и не-UTF8.
    let tmp = Temp::new("large");
    let f = file("big.rs");
    let mut store = tmp.store(3);
    let text: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
    store.put(&f, &text).unwrap();
    let snap = store.latest(&f).unwrap().unwrap();
    assert_eq!(snap.text.len(), text.len(), "длина не совпала");
    assert_eq!(snap.text, text, "содержимое не совпало");
    // Нули в данных тоже должны выжить — это проверка на то, что блоб
    // сохраняется целиком, а не до первого нуля.
    let zeros = (0..500_000u32).filter(|i| i % 251 == 0).count();
    assert_eq!(snap.text.iter().filter(|&&b| b == 0).count(), zeros);
}

#[test]
fn get_by_seq_and_missing_is_none() {
    let tmp = Temp::new("byseq");
    let f = file("a.rs");
    let mut store = tmp.store(3);
    store.put(&f, "первый".as_bytes()).unwrap();
    let seq = store.latest(&f).unwrap().unwrap().seq;
    let by_seq = store.get(&f, seq).unwrap().expect("снапшот по номеру");
    assert_eq!(by_seq.text, "первый".as_bytes());
    assert!(store.get(&f, 999_999).unwrap().is_none());
    assert!(store.latest(&file("нет-такого.rs")).unwrap().is_none());
}

#[test]
fn forget_removes_only_that_file() {
    let tmp = Temp::new("forget");
    let a = file("a.rs");
    let b = file("b.rs");
    let mut store = tmp.store(3);
    store.put(&a, b"a").unwrap();
    store.put(&b, b"b").unwrap();
    store.forget(&a).unwrap();
    assert_eq!(store.count(&a).unwrap(), 0);
    assert_eq!(store.count(&b).unwrap(), 1);
}

#[test]
fn reopening_keeps_existing_snapshots() {
    // Второе открытие не стирает то, что записано ранее.
    let tmp = Temp::new("keep-on-reopen");
    let f = file("a.rs");
    {
        let mut store = tmp.store(3);
        store.put(&f, "из первого запуска".as_bytes()).unwrap();
    }
    {
        let mut store = tmp.store(3);
        assert_eq!(store.count(&f).unwrap(), 1);
        store.put(&f, "из второго запуска".as_bytes()).unwrap();
        let list = store.list(&f).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "из второго запуска".as_bytes());
        assert_eq!(list[1].text, "из первого запуска".as_bytes());
    }
}

#[test]
fn two_stores_on_one_database() {
    // Два независимых подключения к одной базе работают одновременно —
    // это то, ради чего выбрана sqlite, а не база с эксклюзивным локом.
    let tmp = Temp::new("two");
    let f = file("a.rs");
    let mut first = tmp.store(3);
    first.put(&f, "первый".as_bytes()).unwrap();

    let mut second = SnapshotStore::open(&tmp.paths(), 3).unwrap();
    second.put(&f, "второй".as_bytes()).unwrap();

    assert_eq!(first.count(&f).unwrap(), 2, "первый видит запись второго");
    assert_eq!(second.count(&f).unwrap(), 2, "второй видит запись первого");
}

#[test]
fn size_on_disk_is_reported() {
    let tmp = Temp::new("size");
    let f = file("a.rs");
    let mut store = tmp.store(3);
    store.put(&f, &vec![b'x'; 100_000]).unwrap();
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
    let tmp = Temp::new("last-good");
    let store = tmp.store(3);
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
    let tmp = Temp::new("last-good-overwrite");
    let mut store = tmp.store(3);

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
    let tmp = Temp::new("last-good-reopen");
    {
        let mut store = tmp.store(3);
        store.save_last_good("config", "body").unwrap();
    }
    let store = tmp.store(3);
    assert_eq!(store.last_good("config").unwrap().as_deref(), Some("body"));
}

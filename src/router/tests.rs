//! Связка буфера и хранилища через маршрутизатор.
//!
//! Проверяется то, ради чего связка и делается: правки в буфере доходят до
//! снапшотов на диске и переживают перезапуск редактора.

use std::path::PathBuf;

use crate::router::{Session, SessionError};

/// Отдельный каталог под каждый тест: базы не делятся.
struct Temp {
    dir: PathBuf,
}

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("claxis-router-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// Файл документа рядом с базой.
    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn session(&self, keep: u32) -> Session {
        Session::open_at(self.dir.clone(), keep).unwrap()
    }

    /// Сессия с короткой историей: снапшот срабатывает быстро.
    fn short_session(&self, keep: u32, depth: u32) -> Session {
        Session::open_at_with(self.dir.clone(), keep, depth).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Набрать текст, пока глубина истории не кончится и не сработает снапшот.
fn type_until_snapshot(session: &Session, file: &std::path::Path, chunk: &str) -> usize {
    let mut doc = session.open_file(file).unwrap();
    let mut i = 0usize;
    while doc.buffer().snapshots_taken() == 0 {
        let pos = doc.buffer().len();
        doc.buffer().insert(pos, chunk.as_bytes()).unwrap();
        i += 1;
        assert!(i < 100_000, "снапшот так и не сработал");
    }
    i
}

#[test]
fn snapshot_reaches_disk() {
    // Правки доходят до снапшота на диске — ради этого всё и затевалось.
    let tmp = Temp::new("to-disk");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);

    let typed = type_until_snapshot(&session, &file, "abc");
    assert!(typed > 0);

    let snaps = session.store().list(&file).unwrap();
    assert!(!snaps.is_empty(), "снапшот должен оказаться в базе");
    assert_eq!(
        snaps[0].text,
        "abc".repeat(typed - 1).into_bytes(),
        "в базе лежит текст на момент снапшота, то есть без последнего куска"
    );
}

#[test]
fn snapshots_survive_restart() {
    // Главное: второй запуск редактора видит снятое ранее.
    let tmp = Temp::new("restart");
    let file = tmp.file("a.rs");
    let typed = {
        let session = tmp.session(3);
        type_until_snapshot(&session, &file, "xy")
    };
    {
        let session = tmp.session(3);
        let snaps = session.store().list(&file).unwrap();
        assert!(!snaps.is_empty(), "после перезапуска снапшоты должны быть");
        assert_eq!(snaps[0].text.len(), (typed - 1) * 2);
    }
}

#[test]
fn file_text_is_the_source_of_truth() {
    // Снапшот не заменяет файл: документ читается с диска, а не из кеша.
    let tmp = Temp::new("file-truth");
    let file = tmp.file("a.rs");
    std::fs::write(&file, "содержимое файла".as_bytes()).unwrap();

    let session = tmp.session(3);
    let doc = session.open_file(&file).unwrap();
    assert_eq!(doc.text(), "содержимое файла".as_bytes());
    assert_eq!(doc.path(), file.as_path());
}

#[test]
fn missing_file_opens_as_empty() {
    // Файла нет — это новый документ, а не ошибка.
    let tmp = Temp::new("missing");
    let session = tmp.session(3);
    let doc = session.open_file(tmp.file("нет-такого.rs")).unwrap();
    assert!(doc.text().is_empty());
}

#[test]
fn two_documents_write_separate_snapshots() {
    // Два документа в одной сессии не мешают друг другу.
    let tmp = Temp::new("two-docs");
    let a = tmp.file("a.rs");
    let b = tmp.file("b.rs");
    let session = tmp.session(3);

    type_until_snapshot(&session, &a, "a");
    type_until_snapshot(&session, &b, "b");

    let sa = session.store().latest(&a).unwrap().expect("снапшот a");
    let sb = session.store().latest(&b).unwrap().expect("снапшот b");
    assert!(sa.text.starts_with(b"a"));
    assert!(sb.text.starts_with(b"b"));
}

#[test]
fn keep_limit_applies_through_session() {
    // Настройка конфига доходит до кольца в базе.
    let tmp = Temp::new("keep");
    let file = tmp.file("a.rs");
    // Короткая история: снапшот срабатывает каждые несколько записей.
    let session = tmp.short_session(2, 8);

    let mut doc = session.open_file(&file).unwrap();
    for _ in 0..40 {
        let end = doc.buffer().len();
        doc.buffer().insert(end, b"z").unwrap();
    }
    assert!(
        doc.buffer().snapshots_taken() > 1,
        "при глубине 8 за 40 вставок снапшот обязан был сработать"
    );
    drop(doc);

    let count = session.store().count(&file).unwrap();
    assert!(
        count <= 2,
        "в базе должно быть не больше keep снапшотов, а их {count}"
    );
}

#[test]
fn unreadable_file_reports_error() {
    // Каталог на месте файла — это не «нет файла», а ошибка чтения.
    let tmp = Temp::new("unreadable");
    let dir = tmp.dir.join("папка");
    std::fs::create_dir_all(&dir).unwrap();
    let session = tmp.session(3);
    let err = session
        .open_file(&dir)
        .expect_err("чтение каталога должно падать");
    match err {
        SessionError::ReadFile { path, .. } => assert!(path.contains("папка")),
        other => panic!("не та ошибка: {other}"),
    }
}

#[test]
fn buffer_and_store_do_not_know_each_other() {
    // Стык сделан в роутере: крейты не зависят друг от друга.
    // Проверяем, что буфер работает и без приёмника вообще.
    let mut buf = claxis_buffer::Buffer::new("начало".as_bytes());
    for _ in 0..9000 {
        buf.insert(buf.len(), b"x").unwrap();
    }
    assert!(
        buf.snapshots_taken() > 0,
        "снапшоты берутся и без приёмника"
    );
    assert_eq!(buf.read().len(), 9000 + "начало".len());
}

#[test]
fn history_depth_reaches_the_buffer() {
    // Глубина истории из конфига доходит до буфера.
    let tmp = Temp::new("depth");
    let file = tmp.file("a.rs");
    let session = tmp.short_session(3, 4);
    assert_eq!(session.history_depth(), 4);
    let mut doc = session.open_file(&file).unwrap();
    assert_eq!(doc.buffer().history_depth(), 4);
}

#[test]
fn bad_history_depth_reports_error() {
    // Недопустимая глубина — ошибка, а не паника.
    let tmp = Temp::new("bad-depth");
    let file = tmp.file("a.rs");
    let session = tmp.short_session(3, 0);
    let err = session
        .open_file(&file)
        .expect_err("нулевая глубина недопустима");
    assert!(matches!(err, SessionError::Buffer(_)), "получено: {err}");
}

#[test]
fn session_opens_in_standard_cache_path() {
    // Путь по умолчанию — кеш ОС, а не рабочий каталог.
    let paths = claxis_store::StorePaths::from_env();
    let dir = paths.dir.display().to_string();
    assert!(
        dir.ends_with("claxis"),
        "путь должен кончаться на claxis: {dir}"
    );
    assert!(
        dir.contains("cache") || dir.contains("Cache"),
        "это должен быть кеш: {dir}"
    );
}

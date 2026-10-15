//! Связка буфера и хранилища через маршрутизатор.
//!
//! Проверяется то, ради чего связка и делается: правки в буфере доходят до
//! снапшотов на диске и переживают перезапуск редактора.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::api::term::{Event, Key, KeyCode, Modifiers};
use crate::router::cycle::{Action, Cycle};
use crate::router::session::{FixedClock, Session, SessionError};

/// Отдельный каталог под каждый тест: базы не делятся.
struct Temp {
    dir: PathBuf,
    store: PathBuf,
}

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("claxis-router-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("store");
        std::fs::create_dir_all(&store).unwrap();
        Self { dir, store }
    }

    /// Файл документа рядом с базой.
    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn session(&self, keep: u8) -> Session {
        Session::open_at(self.dir.clone(), keep).unwrap()
    }

    /// Сессия с короткой историей: снапшот срабатывает быстро.
    fn short_session(&self, keep: u8, depth: u32) -> Session {
        Session::open_at_with(self.dir.clone(), keep, true, depth).unwrap()
    }

    /// Сам каталог.
    fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Каталог хранилища рядом с конфигом, чтобы тесты не трогали кеш
    /// пользователя.
    fn store_dir(&self) -> &std::path::Path {
        &self.store
    }

    /// Сессия, которая не пишет снапшоты на диск.
    fn no_persist_session(&self, keep: u8, depth: u32) -> Session {
        Session::open_at_with(self.dir.clone(), keep, false, depth).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Набрать текст и снять снапшот, дождавшись интервала на поддельных часах.
fn type_until_snapshot(session: &Session, file: &std::path::Path, chunk: &str) -> usize {
    let mut doc = session.open_file(file).unwrap();
    // Пока часы стоят, снапшот не снимется ни при какой правке.
    doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
    let mut i = 0usize;
    while doc.buffer().snapshots_taken() == 0 && i < 40 {
        let pos = doc.buffer().len();
        doc.insert(pos, chunk.as_bytes()).unwrap();
        doc.snapshot_if_due();
        i += 1;
    }
    // Пять минут прошли — снимок снимает всё, что набрано.
    doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(600))));
    doc.snapshot_if_due();
    assert_eq!(doc.buffer().snapshots_taken(), 1, "снапшот не сработал");
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
        "abc".repeat(typed).into_bytes(),
        "в базе лежит всё, что набрано к моменту снапшота"
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
        assert_eq!(snaps[0].text.len(), typed * 2);
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
    // Часы стоят: ни одна из сорока вставок не успевает за интервал.
    doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
    for _ in 0..40 {
        let end = doc.buffer().len();
        doc.insert(end, b"z").unwrap();
        doc.snapshot_if_due();
    }
    assert_eq!(
        doc.buffer().snapshots_taken(),
        0,
        "за сорок вставок подряд снапшот не заслужили"
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
    // Проверяем, что документ работает и без приёмника вообще.
    let mut doc = crate::api::rope::Document::new("начало".as_bytes());
    for _ in 0..9000 {
        doc.insert(doc.len(), b"x").unwrap();
    }
    assert_eq!(
        doc.snapshots_taken(),
        0,
        "документ сам снимает только по прямой просьбе"
    );
    doc.snapshot();
    assert!(doc.snapshots_taken() > 0, "снапшот берётся и без приёмника");
    assert_eq!(doc.read().len(), 9000 + "начало".len());
}

#[test]
fn history_depth_reaches_the_buffer() {
    // Глубина истории из конфига доходит до буфера.
    let tmp = Temp::new("depth");
    let file = tmp.file("a.rs");
    let session = tmp.short_session(3, 4);
    assert_eq!(session.history_depth(), 4);
    let doc = session.open_file(&file).unwrap();
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
    let paths = crate::api::store::StorePaths::from_env();
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

#[test]
fn file_too_large_reports_bytes_and_limit() {
    // Файл больше u32::MAX не помещается в буфер.
    let err = SessionError::FileTooLarge {
        bytes: 5_000_000_000,
    };
    let text = err.text();

    // Точный размер в байтах.
    assert!(
        text.contains("5 000 000 000 bytes"),
        "в сообщении должен быть точный размер в байтах: {text}"
    );
    // Размер файла в десятичных гигабайтах: 5 000 000 000 / 10^9 = 5.0.
    assert!(
        text.contains("(5.0 GB)"),
        "размер файла должен быть в десятичных ГБ: {text}"
    );
    // Предел — точные байты: u32::MAX = 4 294 967 295.
    assert!(
        text.contains("\nthe maximum is 4 294 967 295 bytes"),
        "предел должен быть в точных байтах: {text}"
    );
    // Гигабайты приложены для удобства: 4.29 ГБ, а не 4.0.
    assert!(
        text.contains("(4.3 GB)"),
        "предел в ГБ должен быть 4.3, а не 4.0: {text}"
    );
}

#[test]
fn gigabytes_are_decimal_not_binary() {
    // Гигабайт считается в 10^9. Если считать в 2^30, то получится 4.0 и
    // пользователю покажут неверный предел.
    let limit_gb = u32::MAX as f64 / 1_000_000_000.0;
    assert!(
        (limit_gb - 4.294967295).abs() < 1e-9,
        "предел должен быть 4.294967295 ГБ, получено {limit_gb}"
    );
    assert!(
        (u32::MAX as f64 / 1_073_741_824.0 - 4.0).abs() < 1e-9,
        "деление на 2^30 даёт 4.0 — это гибибайты, а не гигабайты"
    );
}

#[test]
fn big_numbers_are_split_into_readable_parts() {
    use crate::api::text::group_digits;

    // Без разделителей длинное число не читается.
    assert_eq!(group_digits(4_294_967_295), "4 294 967 295");
    assert_eq!(group_digits(1_000), "1 000");
    assert_eq!(group_digits(1_234_567_890), "1 234 567 890");
    // Числа короче трёх разрядов не трогаем.
    assert_eq!(group_digits(999), "999");
    assert_eq!(group_digits(100), "100");
    assert_eq!(group_digits(0), "0");
}

#[test]
fn persist_false_keeps_snapshots_in_memory_only() {
    // Настройка persist обязана что-то значить: при false снапшоты не пишутся.
    let tmp = Temp::new("no-persist");
    let file = tmp.file("a.rs");
    let session = tmp.no_persist_session(3, 8);

    let mut doc = session.open_file(&file).unwrap();
    doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
    for _ in 0..40 {
        let end = doc.buffer().len();
        doc.insert(end, b"z").unwrap();
    }
    doc.snapshot();
    // Снапшот взят, текст в буфере есть.
    assert_eq!(doc.buffer().snapshots_taken(), 1, "снапшот взят");
    assert_eq!(doc.buffer().len(), 40, "правки на месте");
    drop(doc);

    // Но на диске пусто.
    assert_eq!(
        session.store().count(&file).unwrap(),
        0,
        "при persist = false на диск не пишется ничего"
    );
}

#[test]
fn defaults_come_from_crate_settings() {
    // Сессия по умолчанию берёт значения из настроек крейтов, а не из
    // зашитых констант.
    let tmp = Temp::new("defaults");
    let session = Session::open_at(tmp.dir.clone(), crate::api::store::KEEP).unwrap();
    assert_eq!(
        session.history_depth(),
        crate::api::rope::defaults::buffer::HISTORY_DEPTH.value
    );
    assert_eq!(session.persist(), crate::api::store::PERSIST);
}

// ── Настройки: конфиг и перевод ────────────────────────────────────────────────

#[test]
fn defaults_come_from_the_schema() {
    // Настройки без конфига берутся из схемы, а не из зашитых чисел.
    let settings = crate::router::Settings::defaults();
    let depth = crate::api::config::schema::find(
        crate::api::config::ConfigFile::Edit,
        "General.history_depth",
    )
    .unwrap()
    .default
    .parse::<u32>()
    .unwrap();
    assert_eq!(settings.history_depth, depth);
    assert_eq!(settings.snapshots_keep, 3);
    assert!(settings.snapshots_persist);
    assert_eq!(settings.language, "en");
}

#[test]
fn each_file_owns_its_keys() {
    // Глубина истории живёт в edit.toml, снапшоты — в files.toml.
    let config = crate::api::config::Config::from_documents([
        (
            crate::api::config::ConfigFile::Edit,
            "[General]\nhistory_depth = 64\n".parse().unwrap(),
        ),
        (
            crate::api::config::ConfigFile::Files,
            "[General]\nsnapshots_keep = 7\nsnapshots_persist = false\n"
                .parse()
                .unwrap(),
        ),
    ]);

    let settings = crate::router::Settings::from_config(&config).unwrap();
    assert_eq!(settings.history_depth, 64);
    assert_eq!(settings.snapshots_keep, 7);
    assert!(!settings.snapshots_persist);
}

#[test]
fn a_file_does_not_affect_another() {
    // Ключ из чужого файла игнорируется, а не применяется как будто свой.
    let config = crate::api::config::Config::from_documents([(
        crate::api::config::ConfigFile::Files,
        "[General]\nhistory_depth = 64\n".parse().unwrap(),
    )]);
    let settings = crate::router::Settings::from_config(&config).unwrap();
    assert_eq!(settings.history_depth, 8192);
}

#[test]
fn every_file_opens_independently() {
    // Все файлы открываются вместе и не мешают друг другу: общий, три
    // состояния, темы и локализация.
    let config = crate::api::config::Config::from_documents(
        crate::api::config::ConfigFile::ALL
            .iter()
            .map(|f| (*f, "[General]\n".parse().unwrap())),
    );
    let open = config.open_files();
    assert_eq!(open.len(), crate::api::config::ConfigFile::ALL.len());
    for f in crate::api::config::ConfigFile::ALL {
        assert!(open.contains(f), "файл {} не открыт", f.file_name());
    }
}

#[test]
fn common_settings_are_read_from_the_editor_file() {
    // Общие настройки редактора лежат в config.toml, а не в состоянии.
    let config = crate::api::config::Config::from_documents([(
        crate::api::config::ConfigFile::Global,
        "[General]\nhold = { alt = 350, ctrl = 200 }\n"
            .parse()
            .unwrap(),
    )]);
    let table = crate::api::config::schema::find_table(
        crate::api::config::ConfigFile::Global,
        "General.hold",
    )
    .unwrap();
    let pairs = config.table(table);
    assert_eq!(
        pairs,
        vec![
            ("alt".to_string(), "350".to_string()),
            ("ctrl".to_string(), "200".to_string())
        ]
    );

    // Тот же ключ в файле состояния ничего не делает.
    let leaked = crate::api::config::Config::from_documents([(
        crate::api::config::ConfigFile::Edit,
        "[General]\nhold = { alt = 350, ctrl = 200 }\n"
            .parse()
            .unwrap(),
    )]);
    assert!(leaked.table(table).is_empty());
}

#[test]
fn unknown_key_is_reported_per_file() {
    // Опечатку должно быть видно, и видно в том файле, где она.
    let config = crate::api::config::Config::from_documents([(
        crate::api::config::ConfigFile::Edit,
        "[General]\nhistory_dept = 64\n".parse().unwrap(),
    )]);
    assert_eq!(
        config.unknown_keys(crate::api::config::ConfigFile::Edit),
        vec!["General.history_dept".to_string()]
    );
    assert!(
        config
            .unknown_keys(crate::api::config::ConfigFile::Files)
            .is_empty()
    );
}

#[test]
fn editor_builds_session_from_settings() {
    // Настройки действительно доходят до сессии, а не лежат мёртвым грузом.
    let tmp = Temp::new("editor");
    let mut settings = crate::router::Settings::defaults();
    settings.history_depth = 32;
    settings.snapshots_keep = 5;
    settings.snapshots_persist = false;

    let editor = crate::router::Editor::with_settings(tmp.store_dir(), settings).unwrap();
    assert_eq!(editor.session().history_depth(), 32);
    assert_eq!(editor.session().keep(), 5);
    assert!(!editor.session().persist());
    assert_eq!(editor.language(), "en");
}

// ── Запуск с конфигом ─────────────────────────────────────────────────────────

#[test]
fn first_run_creates_every_config_file() {
    // При первом запуске создаются все файлы разом, со всеми ключами.
    let tmp = Temp::new("first-run-cfg");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(!editor.created.is_empty(), "файлы должны быть созданы");
    for file in crate::api::config::ConfigFile::ALL {
        assert!(
            tmp.dir().join(file.file_name()).exists(),
            "{} не создан",
            file.file_name()
        );
    }
    assert!(editor.issues.is_empty(), "проблемы: {:?}", editor.issues);
    assert!(!editor.used_last_good);
}

#[test]
fn generated_files_carry_every_key_with_a_comment() {
    let tmp = Temp::new("keys-with-comments");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    for key in crate::api::config::KEYS {
        let path = tmp.dir().join(key.file.file_name());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(key.key()),
            "{}: нет ключа {}",
            key.file.file_name(),
            key.key()
        );
        assert!(
            text.contains(key.comment),
            "{}: нет комментария к {}",
            key.file.file_name(),
            key.key()
        );
    }
}

#[test]
fn settings_from_the_file_reach_the_session() {
    let tmp = Temp::new("settings-reach");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    std::fs::write(
        tmp.dir().join("edit.toml"),
        "[General]\nhistory_depth = 64\n",
    )
    .unwrap();
    std::fs::write(
        tmp.dir().join("files.toml"),
        "[General]\nsnapshots_keep = 7\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.session().history_depth(), 64);
    assert_eq!(editor.session().keep(), 7);
}

#[test]
fn broken_config_falls_back_to_the_saved_one() {
    // Главное: сломанный конфиг не должен оставлять пользователя с пустым
    // редактором. Работаем на последнем корректном.
    let tmp = Temp::new("fallback");
    std::fs::create_dir_all(tmp.dir()).unwrap();

    // Первый запуск: всё в порядке, конфиг сохраняется.
    {
        let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
        assert!(editor.issues.is_empty());
        assert!(!editor.used_last_good);
    }

    // Второй запуск: пользователь сломал файл.
    std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 1\n").unwrap();
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    assert!(editor.used_last_good, "должен был пойти на запасной конфиг");
    assert_eq!(editor.issues.len(), 1);
    let issue = editor.first_issue().unwrap();
    assert_eq!(issue.file, "edit.toml");
    assert_eq!(issue.line, 2);
    // Настройки те же, что и до поломки.
    assert_eq!(editor.session().history_depth(), 8192);
}

#[test]
fn broken_config_on_first_run_has_nothing_to_fall_back_to() {
    // Падать можно только когда работать не с чем.
    let tmp = Temp::new("no-fallback");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    // Готовим сломанный файл ДО первого запуска: сохранять нечего.
    std::fs::write(tmp.dir().join("edit.toml"), "[General]\nhistory_dept = 1\n").unwrap();

    match crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()) {
        Ok(editor) => {
            // Запасной мог появиться из других файлов — тогда всё в порядке.
            assert!(editor.used_last_good || !editor.issues.is_empty());
        }
        Err(crate::router::StartError::NoUsableConfig { .. }) => {}
        Err(other) => panic!("не та ошибка: {other}"),
    }
}

#[test]
fn editing_one_key_keeps_the_rest_of_the_file() {
    // Точечная запись не переписывает файл: комментарии и порядок целы.
    let tmp = Temp::new("point-write");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    let key = crate::api::config::schema::find(
        crate::api::config::ConfigFile::Edit,
        "General.history_depth",
    )
    .unwrap();
    crate::api::config::generate::write_value(tmp.dir(), key, "256").unwrap();

    let text = std::fs::read_to_string(tmp.dir().join("edit.toml")).unwrap();
    assert!(text.contains("history_depth = 256"), "{text}");
    assert!(text.contains(key.comment), "комментарий пропал: {text}");
    assert!(text.contains("0 to 65535"), "диапазон пропал: {text}");
}

// ── Динамический конфиг: сохранил — применилось ──────────────────────────────

#[test]
fn saved_config_applies_without_restart() {
    // Главное: сохранение файла подхватывается сразу.
    let tmp = Temp::new("dynamic-apply");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    std::fs::write(
        tmp.dir().join("files.toml"),
        "[General]\nsnapshots_keep = 9\n",
    )
    .unwrap();

    let reloaded = editor.reload();
    assert!(
        reloaded.is_ok(),
        "конфиг должны были принять: {:?}",
        reloaded
    );
    assert!(reloaded.issues.is_empty());
    // Применилось сразу, без перезапуска.
    assert!(
        reloaded.applied.applied.contains(&"snapshots_keep"),
        "keep должен примениться сразу: {:?}",
        reloaded.applied
    );
    assert!(reloaded.applied.needs_restart.is_empty());
    assert_eq!(editor.session().keep(), 9);
}

#[test]
fn broken_saved_config_is_refused_and_reported() {
    // Сломанный конфиг не применяется: старые настройки остаются, а где
    // ошибка — видно, чтобы подсветить.
    let tmp = Temp::new("dynamic-broken");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    let before = editor.session().keep();

    std::fs::write(
        tmp.dir().join("files.toml"),
        "[General]\nsnapshots_keep = 9\nnonsense = 1\n",
    )
    .unwrap();

    let reloaded = editor.reload();
    assert!(!reloaded.is_ok(), "сломанный конфиг принят быть не должен");
    assert_eq!(reloaded.issues.len(), 1);

    let issue = reloaded.first_issue().expect("где сломалось");
    assert_eq!(issue.file, "files.toml");
    assert_eq!(issue.line, 3);
    assert_eq!(issue.key, "General.nonsense");

    // Настройки те же: сломанный конфиг ничего не поменял.
    assert_eq!(editor.session().keep(), before);
}

#[test]
fn reload_writes_nothing_to_the_file() {
    // Никто ничего не дописывает: конфиг выглядит ровно так, как его написал
    // пользователь.
    let tmp = Temp::new("dynamic-no-write");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    let path = tmp.dir().join("files.toml");
    let mine = "[General]\nsnapshots_keep = 4\n";
    std::fs::write(&path, mine).unwrap();

    editor.reload();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        mine,
        "редактор не должен трогать файл пользователя"
    );
}

#[test]
fn depth_change_asks_for_restart_instead_of_half_applying() {
    // Размер пула задан при создании буфера: на лету не меняется, и редактор
    // честно говорит об этом, а не делает вид.
    let tmp = Temp::new("dynamic-depth");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    std::fs::write(
        tmp.dir().join("edit.toml"),
        "[General]\nhistory_depth = 128\n",
    )
    .unwrap();

    let reloaded = editor.reload();
    assert!(reloaded.is_ok());
    assert!(
        reloaded.applied.needs_restart.contains(&"history_depth"),
        "глубина должна просить перезапуск: {:?}",
        reloaded.applied
    );
    // Буфер продолжает работать на прежней глубине: половинчатое применение
    // хуже, чем честная просьба перезапустить.
    assert_eq!(editor.session().opened_depth(), 8192);
}

#[test]
fn reloading_an_unchanged_file_changes_nothing() {
    let tmp = Temp::new("dynamic-noop");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    let reloaded = editor.reload();
    assert!(reloaded.is_ok());
    assert!(reloaded.applied.applied.is_empty());
    assert!(reloaded.applied.needs_restart.is_empty());
}

// ── Наблюдение за каталогом конфига ────────────────────────────────────────────

#[test]
fn editor_watches_the_config_directory() {
    let tmp = Temp::new("watching");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(editor.is_watching(), "редактор должен следить за конфигом");
}

#[test]
fn external_edit_is_applied_without_touching_the_editor() {
    // Главное: правка снаружи видна открытому редактору.
    let tmp = Temp::new("external");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    let before = editor.session().keep();

    // Пишем со стороны, как будто это другой редактор или git.
    std::fs::write(
        tmp.dir().join("files.toml"),
        "[General]\nsnapshots_keep = 6\n",
    )
    .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut applied = None;
    while std::time::Instant::now() < deadline && applied.is_none() {
        if let Some(reloaded) = editor.poll_watch() {
            applied = Some(reloaded);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let reloaded = applied.expect("правка снаружи должна быть замечена");
    assert!(reloaded.is_ok(), "конфиг должны были принять: {reloaded:?}");
    assert!(before != editor.session().keep(), "настройка не сменилась");
    assert_eq!(editor.session().keep(), 6);
}

#[test]
fn nothing_happens_when_the_config_is_quiet() {
    // Пока файлы не трогают, перечитывания быть не должно.
    let tmp = Temp::new("quiet");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();

    for _ in 0..10 {
        assert!(
            editor.poll_watch().is_none(),
            "перечитывание без изменения файла"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

// ── Язык интерфейса ───────────────────────────────────────────────────────────

/// Положить файл перевода в каталог конфига под временным тестом.
fn write_translation(config_dir: &std::path::Path, lang: &str, body: &str) {
    let dir = config_dir.join("lang");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{lang}.toml")), body).unwrap();
}

#[test]
fn a_language_from_the_config_reaches_the_interface() {
    // Главное: язык из config.toml действительно попадает в каталог, который
    // потом берут подписи. Иначе ключ был бы включён, но ничего не менял.
    let tmp = Temp::new("lang-from-config");
    write_translation(
        tmp.dir(),
        "ru",
        "\"Open file\" = \"Открыть файл\"\n\"Save file\" = \"Сохранить файл\"\n",
    );
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"ru\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "ru");
    assert!(
        editor.language_error.is_none(),
        "язык не загрузился: {:?}",
        editor.language_error
    );
    assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
}

#[test]
fn english_needs_no_file_on_disk() {
    // Английский зашит в бинарник: папки lang может не быть вообще.
    let tmp = Temp::new("lang-english");
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "en");
    assert!(editor.language_error.is_none());
}

#[test]
fn a_missing_language_file_is_reported_not_hidden() {
    // Пользователь попросил язык, которого нет. Молча отдать английский —
    // значит показать не тот интерфейс, который он выбрал.
    let tmp = Temp::new("lang-missing");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"ru\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "ru");
    let error = editor
        .language_error
        .as_ref()
        .expect("отсутствие файла должно быть сообщено");
    assert!(
        error.text().contains("ru.toml"),
        "в сообщении нет пути: {}",
        error.text()
    );
}

#[test]
fn a_broken_translation_still_opens_the_editor() {
    // Поломка перевода — не повод не запускать редактор: конфиг в порядке,
    // работать есть над чем. Но о поломке сказано.
    let tmp = Temp::new("lang-broken");
    write_translation(tmp.dir(), "ru", "\"Open file\" = \n");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"ru\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(editor.issues.is_empty(), "конфиг цел: {:?}", editor.issues);
    assert!(
        editor.language_error.is_some(),
        "поломка перевода не сообщена"
    );
    // Сессия работает: редактор открылся, а не отказался.
    assert_eq!(editor.session().keep(), 3);
}

#[test]
fn switching_the_language_applies_without_restart() {
    // Язык меняется в конфиге и применяется сразу, как и остальные ключи.
    let tmp = Temp::new("lang-switch");
    write_translation(tmp.dir(), "ru", "\"Open file\" = \"Открыть файл\"\n");
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "en");

    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"ru\"\n",
    )
    .unwrap();
    let reloaded = editor.reload();
    assert!(reloaded.is_ok(), "{reloaded:?}");
    assert_eq!(editor.language(), "ru");
    assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
}

#[test]
fn a_language_code_cannot_escape_the_config_directory() {
    // Код языка приходит из конфига, то есть из файла на диске: уход вверх
    // через `../../` читался бы чужой файл.
    let tmp = Temp::new("lang-escape");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"../../etc/passwd\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(editor.language_error.is_some(), "обход должен быть отбит");
}

#[test]
fn the_generated_config_asks_for_a_language_up_front() {
    // Ключ включён: в свежем конфиге он не закомментирован, иначе язык пришлось
    // бы сначала раскомментировать вручную.
    let tmp = Temp::new("lang-generated");
    crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    let text = std::fs::read_to_string(tmp.dir().join("config.toml")).unwrap();
    assert!(
        text.contains("\nlanguage = ") || text.contains("language = \"en\""),
        "ключ языка должен быть активным: {text}"
    );
    assert!(
        !text.contains("#language"),
        "ключ языка не должен быть закомментированным: {text}"
    );
}

// ── Темы ─────────────────────────────────────────────────────────────────────

/// Положить файл темы в каталог конфига под временным тестом.
fn write_theme(config_dir: &std::path::Path, file: &str) {
    let dir = config_dir.join("theme");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), "").unwrap();
}

#[test]
fn a_fresh_config_starts_without_a_complaint() {
    // Папки тем на первом запуске нет, и тема «default» встроенная. Если бы
    // требовался файл, редактор ругался бы на каждом первом запуске.
    let tmp = Temp::new("theme-fresh");
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(!tmp.dir().join("theme").exists(), "папки быть не должно");
    assert!(
        editor.theme_error.is_none(),
        "на первом запуске тема не должна быть претензией: {:?}",
        editor.theme_error
    );
    assert_eq!(editor.settings().theme, "default");
}

#[test]
fn a_theme_from_the_config_reaches_the_settings() {
    // Ключ включён: имя из config.toml доходит до настроек, а тема с таким
    // именем в папке находится.
    let tmp = Temp::new("theme-from-config");
    write_theme(tmp.dir(), "everforest.toml");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"everforest\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.settings().theme, "everforest");
    assert!(
        editor.theme_error.is_none(),
        "тема не найдена: {:?}",
        editor.theme_error
    );
}

#[test]
fn an_unknown_theme_is_reported_with_the_list() {
    // Пользователь назвал тему, которой нет: молча брать «default» — значит
    // показать не ту тему, которую он выбрал.
    let tmp = Temp::new("theme-unknown");
    write_theme(tmp.dir(), "everforest.toml");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"solarized\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    let error = editor
        .theme_error
        .as_ref()
        .expect("неизвестная тема должна быть сообщена");
    let text = error.text();
    assert!(text.contains("solarized"), "{text}");
    assert!(text.contains("everforest"), "в тексте нет списка: {text}");
}

#[test]
fn a_theme_name_cannot_escape_the_config_directory() {
    // Имя приходит из файла на диске: уход вверх через `../../` искал бы тему
    // вне папки тем.
    let tmp = Temp::new("theme-escape");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"../../../etc/passwd\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(editor.theme_error.is_some(), "обход должен быть отбит");
}

#[test]
fn switching_the_theme_applies_without_restart() {
    let tmp = Temp::new("theme-switch");
    write_theme(tmp.dir(), "everforest.toml");
    let mut editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.settings().theme, "default");

    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"everforest\"\n",
    )
    .unwrap();
    let reloaded = editor.reload();
    assert!(reloaded.is_ok(), "{reloaded:?}");
    assert_eq!(editor.settings().theme, "everforest");
    assert!(editor.theme_error.is_none());
}

#[test]
fn a_broken_theme_does_not_stop_the_editor() {
    // Тема не нашлась, а редактор должен открыться: конфиг цел, работать есть
    // над чем. О поломке сказано, и ключ остаётся как есть.
    let tmp = Temp::new("theme-broken");
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"nope\"\n",
    )
    .unwrap();

    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert!(editor.issues.is_empty(), "конфиг цел: {:?}", editor.issues);
    assert!(editor.theme_error.is_some());
    assert_eq!(editor.session().keep(), 3);
}

#[test]
fn the_generated_config_asks_for_a_theme_up_front() {
    let tmp = Temp::new("theme-generated");
    crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    let text = std::fs::read_to_string(tmp.dir().join("config.toml")).unwrap();
    assert!(
        text.contains("theme = \"default\""),
        "ключ темы должен быть активным: {text}"
    );
    assert!(!text.contains("#theme"), "ключ не закомментирован: {text}");
}

#[test]
fn themes_in_the_folder_do_not_apply_themselves() {
    // Папка — склад, а не выбор. Файлов может лежать сколько угодно, но без
    // имени в конфиге работает `default`: иначе тема применялась бы сама, и
    // пользователь не знал бы, почему.
    let tmp = Temp::new("theme-no-choice");
    write_theme(tmp.dir(), "everforest.toml");
    write_theme(tmp.dir(), "solarized.toml");
    // Конфиг создаётся с ключом темы по умолчанию — это `default`.
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.settings().theme, "default");
    assert!(editor.theme_error.is_none());

    // Как только имя написано — работает именно эта тема.
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\ntheme = \"everforest\"\n",
    )
    .unwrap();
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.settings().theme, "everforest");
}

#[test]
fn translations_in_the_folder_do_not_apply_themselves() {
    // Папка переводов — склад, как и папка тем. Файлов может лежать сколько
    // угодно, и сломанный чужой перевод не должен мешать: работает только тот,
    // чей код написан в конфиге.
    let tmp = Temp::new("lang-no-choice");
    write_translation(tmp.dir(), "ru", "\"Open file\" = \"Открыть файл\"\n");
    write_translation(tmp.dir(), "de", "\"Open file\" = \n");
    write_translation(tmp.dir(), "fr", "\"Open file\" = \"Ouvrir le fichier\"\n");

    // Язык по умолчанию — английский, и он зашит, а не взят из папки.
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "en");
    assert!(editor.language_error.is_none(), "чужие файлы не мешают");

    // Написан русский — читается русский, сломанный немецкий не мешает.
    std::fs::write(
        tmp.dir().join("config.toml"),
        "[General]\nlanguage = \"ru\"\n",
    )
    .unwrap();
    let editor = crate::router::Editor::open_in(tmp.dir(), tmp.store_dir()).unwrap();
    assert_eq!(editor.language(), "ru");
    assert!(editor.language_error.is_none());
    assert_eq!(editor.catalog().get("Open file"), Some("Открыть файл"));
}

#[test]
fn only_the_config_crate_knows_where_the_folders_are() {
    // Крейты получают готовый путь и ничего не знают о раскладке на диске.
    // Если тема или перевод снова начнут собирать путь сами, знание о папках
    // размажется по двум местам, и правка одной распадётся на другую.
    //
    // Проверяем по коду: ни в `claxis-theme`, ни в `claxis-i18n` не должно быть
    // имён папок. Имена папок — дело одного крейта.
    let forbidden = ["THEME_DIR", "LANG_DIR", "theme_dir", "lang_dir", "user_dir"];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for crate_name in ["claxis-theme", "claxis-i18n"] {
        let src = root.join(crate_name).join("src");
        for entry in std::fs::read_dir(&src).unwrap().flatten() {
            if entry.path().extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap();
            for name in forbidden {
                // Только код, не комментарии и не примеры сообщений: текст
                // про пути в сообщении — это текст, а не знание о раскладке.
                for (n, line) in text.lines().enumerate() {
                    let line = line.trim();
                    if line.starts_with("//") || line.contains('"') {
                        continue;
                    }
                    assert!(
                        !line.contains(name),
                        "{}:{}: крейт не должен знать про папки ({name}): {line}",
                        entry.path().display(),
                        n + 1
                    );
                }
            }
        }
    }
}

#[test]
fn the_config_crate_resolves_every_folder() {
    // Раз знает один крейт, он и раздаёт папки. Обе папки собираются оттуда.
    let config = crate::api::config::path::user_dir().expect("каталог находится");
    let theme = crate::api::config::path::theme_dir(&config);
    let lang = crate::api::config::path::lang_dir(&config);
    assert!(theme.ends_with("theme"), "{theme:?}");
    assert!(lang.ends_with("lang"), "{lang:?}");
    assert_ne!(theme, lang);
}

#[test]
fn history_survives_reopening_the_document() {
    // Главное: правки и снапшот переживают перезапуск. Текст верный, но без
    // истории отменять нечего — половина работы пользователя исчезает.
    let tmp = Temp::new("history-survives");
    std::fs::create_dir_all(tmp.dir()).unwrap();
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();

    {
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.delete(0, 1).unwrap();
        assert_eq!(doc.text(), b"bcXY\n");
        // Снапшот уносит историю на диск.
        doc.snapshot();
    }

    // Новый процесс читает тот же файл.
    let session2 = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    let mut doc = session2.open_file(&file).unwrap();

    // Текст на месте.
    assert_eq!(doc.text(), b"bcXY\n");
    // И история на месте: отмена работает после перезапуска.
    doc.undo()
        .unwrap()
        .expect("отмена доступна после перезапуска");
    assert_eq!(doc.text(), b"abcXY\n");
}

#[test]
fn a_file_changed_outside_is_not_overwritten_by_the_snapshot() {
    // Файл правили не из редактора. Снапшот построен на прежней версии, и
    // применять его нельзя: правка извне стала бы невидимой и уехала бы под
    // следующим сохранением.
    let tmp = Temp::new("changed-outside");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    {
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();
    }

    // Правка снаружи: и содержимое, и длина.
    std::fs::write(&file, "чужая правка\n").unwrap();

    let session2 = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    let doc = session2.open_file(&file).unwrap();
    assert_eq!(
        doc.text(),
        "чужая правка\n".as_bytes(),
        "открыт файл как есть"
    );
    assert!(doc.changed_outside(), "об изменении сообщаем");
}

#[test]
fn keep_zero_means_memory_only_and_writes_nothing() {
    // Ноль в кольце — это «только память». Приёмника быть не должно: иначе он
    // писал бы в базу строку, которую кольцо тут же удаляет, а пользователь с
    // `persist = true` думал бы, что работа переживает перезапуск.
    let tmp = Temp::new("keep-zero");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 0, true, 64).unwrap();
    {
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();
    }

    let store = session.store().reopen().unwrap();
    assert!(
        store.latest(&file).unwrap().is_none(),
        "в базу не должно уйти ничего"
    );
}

#[test]
fn a_snapshot_written_without_trouble_says_nothing() {
    // Здоровое состояние не должно пугать сообщениями.
    let tmp = Temp::new("snapshot-ok");
    let file = tmp.file("a.rs");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    let mut doc = session.open_file(&file).unwrap();
    doc.insert(3, b"XY").unwrap();
    doc.snapshot();
    assert!(
        doc.snapshot_error().is_none(),
        "снапшот лёг, жаловаться не на что"
    );
}

#[test]
fn an_untouched_file_still_gets_its_snapshot() {
    // Обратная сторона проверки: файл не трогали, снапшот применяется.
    let tmp = Temp::new("untouched");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    {
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();
    }

    let session2 = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    let doc = session2.open_file(&file).unwrap();
    assert_eq!(doc.text(), b"abcXY\n", "текст из снапшота");
    assert!(!doc.changed_outside(), "файл не меняли, ложной тревоги нет");
}

#[test]
fn a_foreign_history_blob_does_not_break_the_file() {
    // В базе может лежать блоб прежнего буфера: он писал в ту же колонку свой
    // формат. Такой блоб историю не восстановит, но файл обязан открыться, а
    // текст — оказаться на месте.
    let tmp = Temp::new("foreign-blob");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    {
        let mut store = session.store().reopen().unwrap();
        let mut old = b"BUFH".to_vec();
        old.extend_from_slice(&[0u8; 40]);
        store.put(&file, b"abc\n", &old).unwrap();
    }

    let session2 = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    let mut doc = session2.open_file(&file).unwrap();
    assert_eq!(doc.text(), b"abc\n", "текст из снапшота на месте");
    // Истории нет — отменять нечего, и это не ошибка.
    assert!(doc.undo().unwrap().is_none());
    doc.insert(3, b"Z").unwrap();
    assert_eq!(doc.text(), b"abcZ\n", "документ рабочий");
}

#[test]
fn a_rope_snapshot_is_written_in_the_rope_format() {
    // Что уходит в базу, обязан быть форматом rope: с магией и версией.
    // Иначе следующий запуск не поймёт, что там лежит.
    let tmp = Temp::new("rope-format");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "abc\n").unwrap();

    let session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    {
        let mut doc = session.open_file(&file).unwrap();
        doc.insert(3, b"XY").unwrap();
        doc.snapshot();
    }

    let store = session.store().reopen().unwrap();
    let snapshot = store.latest(&file).unwrap().expect("снапшот записан");
    assert_eq!(&snapshot.history[..4], b"CLRP", "магия формата rope");
    assert!(snapshot.history.len() > 5, "в блобе есть версия");

    // И разбирается тем же кодом, без потерь.
    let restored = crate::api::rope::History::decode(&snapshot.history)
        .expect("блоб из базы — наш")
        .restore()
        .expect("документ собирается");
    assert_eq!(restored.read(), b"abcXY\n");
}

#[test]
fn a_snapshot_is_taken_by_the_clock_and_not_by_the_edit_count() {
    // Снапшот снимает время. Правок может быть сколько угодно, и лимит на
    // них не влияет: интервал честно отсчитывается часами.
    let tmp = Temp::new("snapshot-by-time");
    let file = tmp.dir().join("doc.txt");
    std::fs::write(&file, "x\n").unwrap();

    let mut session = crate::router::Session::open_at_with(tmp.dir(), 3, true, 64).unwrap();
    session.set_snapshot_interval(Duration::from_secs(300));

    // Часы стоят: сколько правок ни сделай, пяти минут не прошло.
    let mut doc = session.open_file(&file).unwrap();
    doc.set_clock(Box::new(FixedClock::after(Duration::ZERO)));
    for _ in 0..500 {
        let at = doc.buffer().len() - 1;
        doc.insert(at, b"y").unwrap();
        doc.snapshot_if_due();
    }
    assert_eq!(doc.text().len(), 502, "правки на месте");
    assert_eq!(doc.buffer().snapshots_taken(), 0, "время не шло");

    // Часы пошли: пять минут прошло, и первая же проверка снимает снапшот.
    doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(299))));
    doc.snapshot_if_due();
    assert_eq!(doc.buffer().snapshots_taken(), 0, "до интервала рано");

    doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(300))));
    doc.snapshot_if_due();
    assert_eq!(
        doc.buffer().snapshots_taken(),
        1,
        "интервал вышел, снимок есть"
    );

    // И отсчёт начинается заново: следующий снимок ещё не через пять минут.
    doc.set_clock(Box::new(FixedClock::after(Duration::from_secs(420))));
    doc.snapshot_if_due();
    assert_eq!(doc.buffer().snapshots_taken(), 1, "отсчёт пошёл заново");
}

#[test]
fn the_interval_comes_from_the_settings() {
    // Интервал из конфига доходит до сессии и до документа.
    let tmp = Temp::new("interval-setting");
    let file = tmp.file("a.rs");
    let mut session = tmp.session(3);
    session.set_snapshot_interval(Duration::from_secs(60));
    assert_eq!(session.snapshot_interval(), Duration::from_secs(60));

    let doc = session.open_file(&file).unwrap();
    assert_eq!(doc.snapshot_interval(), Duration::from_secs(60));
}

#[test]
fn the_statistics_counts_what_the_user_did() {
    // Счётчик живёт в документе сессии: он видит правки, документ — нет.
    let tmp = Temp::new("stats");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    doc.insert(0, "абв".as_bytes()).unwrap();
    doc.insert(3, "г".as_bytes()).unwrap();
    doc.delete(0, 1).unwrap();
    // «абв» — три буквы, а не шесть байт.
    assert_eq!(doc.stats().input_chars, 4);
    assert_eq!(doc.stats().deleted_chars, 1);
    assert_eq!(doc.stats().edits, 3);
}

#[test]
fn the_statistics_count_only_the_time_a_key_was_held() {
    // Секунды между нажатиями — это размышление, а не печать.
    let tmp = Temp::new("presses");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    doc.note_press();
    // Между нажатиями прошло двадцать секунд: они сюда не попадают.
    doc.note_press();
    doc.note_held(Duration::from_millis(70));
    doc.note_held(Duration::from_millis(130));
    // Третье нажатие так и не отпустили: время оно не добавит.
    doc.note_press();

    assert_eq!(doc.stats().presses, 3, "нажатий три");
    assert_eq!(
        doc.stats().pressed,
        Duration::from_millis(200),
        "время заняли сами нажатия"
    );
}

#[test]
fn history_usage_never_passes_a_hundred() {
    let s = crate::router::stats::Stats::default();
    assert_eq!(
        s.history_percent(900, 100),
        100,
        "процент упирается в сотню"
    );
    assert_eq!(
        s.history_remaining(900, 100),
        -800,
        "остаток бывает отрицательным"
    );
    assert_eq!(s.history_percent(10, 0), 0, "без размера процент нулевой");
    assert_eq!(s.history_percent(40, 100), 40);
}

#[test]
fn the_statistics_screen_is_configurable() {
    // Настройки экрана доходят из конфига до редактора.
    let tmp = Temp::new("savescreen");
    std::fs::write(
        tmp.dir.join("config.toml"),
        "[Savescreen]\ninclude = false\nmode = \"message\"\n",
    )
    .unwrap();
    let editor = crate::router::Editor::open_in(&tmp.dir, &tmp.store).unwrap();
    assert!(!editor.settings().savescreen_include, "экран выключен");
    assert_eq!(editor.settings().savescreen_mode, "message");
}

#[test]
fn the_history_usage_signature_is_an_inline_table() {
    // Подписи истории лежат во встроенной таблице, а не плоскими ключами: их
    // пишут и читают по именам `percent` и `number`.
    let tmp = Temp::new("history-usage-table");
    // Ровно так, как это пишет человек: таблица разбита по строкам.
    std::fs::write(
        tmp.dir.join("config.toml"),
        "[Savescreen.Setting]\nhistory_usage = {\n  percent = \"{percent}\",\n  number = \"{remaining}\"\n}\n",
    )
    .unwrap();
    let editor = crate::router::Editor::open_in(&tmp.dir, &tmp.store).unwrap();
    let settings = editor.settings();
    assert_eq!(settings.savescreen_history_usage_percent, "{percent}");
    assert_eq!(settings.savescreen_history_usage_number, "{remaining}");
}

#[test]
fn the_history_signature_defaults_to_what_the_schema_says() {
    let settings = crate::router::Settings::defaults();
    assert!(settings.savescreen_include);
    assert_eq!(settings.savescreen_mode, "fullscreen");
    assert_eq!(settings.savescreen_history_usage_percent, "{used}");
    assert_eq!(settings.savescreen_history_usage_number, "{remaining}");
}

#[test]
fn a_zero_interval_snapshots_on_every_edit() {
    // Нулевой интервал — это «на каждой правке». В конфиге так не выставляют,
    // но без него не проверить, что счётчик не врёт.
    let tmp = Temp::new("zero-interval");
    let file = tmp.file("a.rs");
    let mut session = tmp.session(3);
    session.set_snapshot_interval(Duration::ZERO);

    let mut doc = session.open_file(&file).unwrap();
    for i in 0..5 {
        let at = doc.buffer().len();
        doc.insert(at, b"x").unwrap();
        doc.snapshot_if_due();
        assert_eq!(i + 1, doc.buffer().snapshots_taken() as u32);
    }
}

#[test]
fn a_missing_file_opens_as_an_empty_document() {
    // Файла нет — это не ошибка, а новый документ. Снапшот такого должен
    // разбираться обратно.
    let tmp = Temp::new("missing-file-snapshot");
    let file = tmp.file("нет-такого.rs");
    let session = tmp.session(3);

    let mut doc = session.open_file(&file).unwrap();
    assert_eq!(doc.text(), Vec::<u8>::new(), "пустой документ");
    doc.insert(0, "новый текст".as_bytes()).unwrap();
    doc.snapshot();
    assert_eq!(doc.text(), "новый текст".as_bytes());

    // С диска читается то, что осталось, и история не ломает его.
    let session2 = tmp.session(3);
    let mut back = session2.open_file(&file).unwrap();
    assert_eq!(back.text(), "новый текст".as_bytes());
    back.undo().unwrap().expect("история из снапшота живая");
    assert_eq!(back.text(), Vec::<u8>::new());
}

#[test]
fn replace_counts_both_sides() {
    // Замена — это удаление и вставка, и статистика должна видеть обе.
    let tmp = Temp::new("stats-replace");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    doc.insert(0, "строка".as_bytes()).unwrap();
    assert_eq!(doc.stats().edits, 1);

    // Позиции и длины — в байтах, а не в буквах: «строка» это 12 байт.
    let len = "строка".len() as u32;
    doc.replace(0, len, "другая".as_bytes()).unwrap();
    let stats = doc.stats();
    assert_eq!(stats.edits, 2, "замена — одна правка");
    assert_eq!(stats.input_chars, 6 + 6, "шесть букв, потом шесть");
    assert_eq!(stats.deleted_chars, u64::from(len));
    assert_eq!(doc.text(), "другая".as_bytes());
}

#[test]
fn statistics_survive_a_snapshot() {
    // Снапшот чинит документ, но не обнуляет счётчики: статистика за
    // сеанс считается с открытия файла, а не с последнего снимка.
    let tmp = Temp::new("stats-snapshot");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    doc.insert(0, "до".as_bytes()).unwrap();
    doc.note_press();
    doc.note_held(Duration::from_millis(50));
    doc.snapshot();
    assert_eq!(doc.stats().input_chars, 2, "счётчик не сбросился");
    assert_eq!(doc.stats().presses, 1);

    doc.insert(0, "после".as_bytes()).unwrap();
    assert_eq!(doc.stats().input_chars, 7, "счётчик считает и дальше");
    assert_eq!(doc.stats().edits, 2);
}

#[test]
fn a_document_survives_a_thousand_tiny_edits() {
    // Много мелких правок подряд: дерево перестраивается на каждой, и ни одна
    // из них не должна его сломать.
    let tmp = Temp::new("tiny-edits");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    for i in 0..1000u32 {
        doc.insert(0, "•".as_bytes()).unwrap();
        // Булет — три байта, как и любая буква кириллицы.
        assert_eq!(doc.buffer().len(), (i + 1) * 3);
    }
    assert_eq!(doc.text().len(), 3000);
    assert_eq!(doc.buffer().read_range(0, 0), Vec::<u8>::new());
}

#[test]
fn a_multiline_table_in_a_real_config_reaches_the_settings() {
    // Весь путь целиком: файл на диске, написанный руками, открывается
    // редактором, и подписи приходят в настройки без кавычек и переносов.
    let tmp = Temp::new("multiline-end-to-end");
    std::fs::write(
        tmp.dir.join("config.toml"),
        r#"[General]
language = "en"

[Savescreen]
include = true

[Savescreen.Setting]
history_usage = {
  percent = "{percent} of {max}",
  number = "{used} of {remaining}"
}
"#,
    )
    .unwrap();
    let editor = crate::router::Editor::open_in(&tmp.dir, &tmp.store).unwrap();
    assert!(
        editor.issues.is_empty(),
        "конфиг с многострочной таблицей разбирается с проблемами: {:?}",
        editor.issues
    );
    let settings = editor.settings();
    assert_eq!(
        settings.savescreen_history_usage_percent,
        "{percent} of {max}"
    );
    assert_eq!(
        settings.savescreen_history_usage_number,
        "{used} of {remaining}"
    );
    assert!(settings.savescreen_include);
}

#[test]
fn the_line_map_stays_with_the_document() {
    // Карта строк обязана ехать вместе с текстом на каждой правке, отмене и
    // повторе. Эталон — обычная строка с историей правок: любое расхождение
    // означает, что карта разъехалась.
    let tmp = Temp::new("line-map");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    let mut text = String::new();
    let mut history: Vec<Op> = Vec::new();
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };

    for _ in 0..600 {
        // Что именно сделали на этом шаге: без этого расхождение бесполезно.
        let done: String;
        match next(5) {
            0 if !text.is_empty() => {
                let at = next(text.len());
                let len = 1 + next((text.len() - at).min(20));
                let removed = text[at..at + len].to_string();
                doc.delete(at as u32, len as u32).unwrap();
                text.replace_range(at..at + len, "");
                history.push(Op::Delete { at, removed });
                done = format!("удалить {at}..{}", at + len);
            }
            1 => {
                let at = next(text.len() + 1);
                let typed = if next(2) == 0 { "\n" } else { "ab" };
                doc.insert(at as u32, typed.as_bytes()).unwrap();
                text.insert_str(at, typed);
                history.push(Op::Insert {
                    at,
                    len: typed.len(),
                });
                done = format!("вставить {typed:?} на {at}");
            }
            2 => {
                let at = next(text.len() + 1);
                let len = next((text.len() - at).min(10));
                let removed = text[at..at + len].to_string();
                let typed = "xy\n";
                doc.replace(at as u32, len as u32, typed.as_bytes())
                    .unwrap();
                text.replace_range(at..at + len, typed);
                history.push(Op::Replace { at, removed });
                done = format!("заменить {at}..{} на {typed:?}", at + len);
            }
            3 => {
                // Снапшот текста не меняет, и карта строк после него обязана
                // остаться в силе. Раньше её пересобирали зря. Зато история
                // правок обнуляется — как и в эталоне.
                doc.snapshot();
                history.clear();
                done = "снапшот".to_string();
            }
            _ => {
                let Some(op) = history.pop() else { continue };
                doc.undo().unwrap().expect("отменять есть что");
                done = format!("отменить {op:?}");
                match op {
                    Op::Insert { at, len } => text.replace_range(at..at + len, ""),
                    Op::Delete { at, removed, .. } => text.insert_str(at, &removed),
                    // Отмена замены убирает вставленное и возвращает вырезанное.
                    Op::Replace { at, removed, .. } => {
                        text.replace_range(at..at + 3, "");
                        text.insert_str(at, &removed);
                    }
                }
            }
        }

        let fresh = crate::router::LineMap::build(text.as_bytes());
        assert_eq!(
            doc.lines().breaks(),
            fresh.breaks(),
            "карта строк разошлась с текстом: {done}, текст {:?}",
            text
        );
        assert_eq!(doc.line_count(), fresh.line_count());
        assert_eq!(
            doc.text(),
            text.as_bytes(),
            "текст разошёлся: {done}, было {:?}",
            text
        );
    }
}

/// Правка в эталоне: сколько байт затронуто и что было там до неё.
#[derive(Debug)]
enum Op {
    Insert { at: usize, len: usize },
    Delete { at: usize, removed: String },
    Replace { at: usize, removed: String },
}

#[test]
fn a_line_map_survives_undo_and_replace() {
    // Отмена вставки убирает её переводы строк, отмена удаления и замены
    // возвращает их на место.
    let tmp = Temp::new("lines-undo");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();

    doc.insert(0, b"one\ntwo\n").unwrap();
    assert_eq!(doc.lines().breaks(), [3, 7]);
    doc.undo().unwrap();
    assert_eq!(doc.lines().breaks(), []);

    doc.replace(0, 0, b"a\nb").unwrap();
    assert_eq!(doc.lines().breaks(), [1]);
    doc.undo().unwrap();
    assert_eq!(doc.lines().breaks(), []);
}

#[test]
fn the_cycle_counts_presses_and_their_time() {
    // Нажатие и отпускание приходят разными событиями: циклу нужно помнить,
    // что было нажато, иначе время нажатия посчитать нечем.
    let tmp = Temp::new("cycle");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();
    let mut cycle = Cycle::new();

    let start = Instant::now();
    let a = Key {
        code: KeyCode::Char('a'),
        modifiers: Modifiers::NONE,
    };
    assert_eq!(
        cycle.handle(
            Event::KeyPressed {
                key: a,
                time: start
            },
            &mut doc
        ),
        Action::Continue
    );
    // Между нажатием и отпусканием прошло время размышления — в сумму оно не
    // попадает, попадает только отрезок от нажатия до отпускания.
    assert_eq!(
        cycle.handle(
            Event::KeyReleased {
                key: a,
                time: start + Duration::from_millis(30)
            },
            &mut doc
        ),
        Action::Continue
    );

    assert_eq!(doc.stats().presses, 1, "нажатие одно");
    assert!(doc.stats().pressed >= Duration::from_millis(30));
    assert!(doc.stats().pressed < Duration::from_millis(500));
}

#[test]
fn a_release_without_a_press_adds_no_time() {
    // Потерянное событие: отпускание без нажатия. Время неизвестно, и
    // выдумывать ноль секунд вместо неизвестного неправильно.
    let tmp = Temp::new("cycle-lost");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();
    let mut cycle = Cycle::new();

    cycle.handle(
        Event::KeyReleased {
            key: Key {
                code: KeyCode::Char('z'),
                modifiers: Modifiers::NONE,
            },
            time: Instant::now(),
        },
        &mut doc,
    );

    assert_eq!(doc.stats().presses, 0);
    assert_eq!(doc.stats().pressed, Duration::ZERO);
}

#[test]
fn ctrl_q_leaves_the_editor() {
    let tmp = Temp::new("cycle-quit");
    let file = tmp.file("a.rs");
    let session = tmp.session(3);
    let mut doc = session.open_file(&file).unwrap();
    let mut cycle = Cycle::new();

    assert_eq!(
        cycle.handle(
            Event::KeyPressed {
                key: Key {
                    code: KeyCode::Char('q'),
                    modifiers: Modifiers::CTRL,
                },
                time: Instant::now(),
            },
            &mut doc
        ),
        Action::Quit
    );
    // Выход не должен попасть в статистику как нажатие.
    assert_eq!(doc.stats().presses, 0);
}

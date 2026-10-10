//! Связка буфера и хранилища через маршрутизатор.
//!
//! Проверяется то, ради чего связка и делается: правки в буфере доходят до
//! снапшотов на диске и переживают перезапуск редактора.

use std::path::PathBuf;

use crate::router::{Session, SessionError};

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

    fn session(&self, keep: u32) -> Session {
        Session::open_at(self.dir.clone(), keep).unwrap()
    }

    /// Сессия с короткой историей: снапшот срабатывает быстро.
    fn short_session(&self, keep: u32, depth: u32) -> Session {
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
    fn no_persist_session(&self, keep: u32, depth: u32) -> Session {
        Session::open_at_with(self.dir.clone(), keep, false, depth).unwrap()
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
    use claxis_text::group_digits;

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
    for _ in 0..40 {
        let end = doc.buffer().len();
        doc.buffer().insert(end, b"z").unwrap();
    }
    // Снапшоты берутся, текст в буфере есть.
    assert!(
        doc.buffer().snapshots_taken() > 1,
        "снапшоты должны браться"
    );
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
        crate::api::buffer::HISTORY_DEPTH.value
    );
    assert_eq!(session.persist(), crate::api::store::PERSIST);
}

// ── Настройки: конфиг и перевод ────────────────────────────────────────────────

#[test]
fn defaults_come_from_the_schema() {
    // Настройки без конфига берутся из схемы, а не из зашитых чисел.
    let settings = crate::router::Settings::defaults();
    let depth =
        claxis_config::schema::find(claxis_config::ConfigFile::Edit, "General.history_depth")
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
    let config = claxis_config::Config::from_documents([
        (
            claxis_config::ConfigFile::Edit,
            "[General]\nhistory_depth = 64\n".parse().unwrap(),
        ),
        (
            claxis_config::ConfigFile::Files,
            "[General]\nsnapshots_keep = 7\nsnapshots_persist = false\n"
                .parse()
                .unwrap(),
        ),
    ]);

    let settings = crate::router::Settings::from_config(&config);
    assert_eq!(settings.history_depth, 64);
    assert_eq!(settings.snapshots_keep, 7);
    assert!(!settings.snapshots_persist);
}

#[test]
fn a_file_does_not_affect_another() {
    // Ключ из чужого файла игнорируется, а не применяется как будто свой.
    let config = claxis_config::Config::from_documents([(
        claxis_config::ConfigFile::Files,
        "[General]\nhistory_depth = 64\n".parse().unwrap(),
    )]);
    let settings = crate::router::Settings::from_config(&config);
    assert_eq!(settings.history_depth, 8192);
}

#[test]
fn every_file_opens_independently() {
    // Все файлы открываются вместе и не мешают друг другу: общий, три
    // состояния, темы и локализация.
    let config = claxis_config::Config::from_documents(
        claxis_config::ConfigFile::ALL
            .iter()
            .map(|f| (*f, "[General]\n".parse().unwrap())),
    );
    let open = config.open_files();
    assert_eq!(open.len(), claxis_config::ConfigFile::ALL.len());
    for f in claxis_config::ConfigFile::ALL {
        assert!(open.contains(f), "файл {} не открыт", f.file_name());
    }
}

#[test]
fn common_settings_are_read_from_the_editor_file() {
    // Общие настройки редактора лежат в config.toml, а не в состоянии.
    let config = claxis_config::Config::from_documents([(
        claxis_config::ConfigFile::Global,
        "[General]\ntap_hold_milliseconds = 350\n".parse().unwrap(),
    )]);
    let key = claxis_config::schema::find(
        claxis_config::ConfigFile::Global,
        "General.tap_hold_milliseconds",
    )
    .unwrap();
    assert_eq!(config.u32_or_default(key), 350);

    // Тот же ключ в файле состояния ничего не делает.
    let leaked = claxis_config::Config::from_documents([(
        claxis_config::ConfigFile::Edit,
        "[General]\ntap_hold_milliseconds = 350\n".parse().unwrap(),
    )]);
    assert_eq!(leaked.u32_or_default(key), 200);
}

#[test]
fn unknown_key_is_reported_per_file() {
    // Опечатку должно быть видно, и видно в том файле, где она.
    let config = claxis_config::Config::from_documents([(
        claxis_config::ConfigFile::Edit,
        "[General]\nhistory_dept = 64\n".parse().unwrap(),
    )]);
    assert_eq!(
        config.unknown_keys(claxis_config::ConfigFile::Edit),
        vec!["General.history_dept".to_string()]
    );
    assert!(
        config
            .unknown_keys(claxis_config::ConfigFile::Files)
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
    for file in claxis_config::ConfigFile::ALL {
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

    for key in claxis_config::KEYS {
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

    let key = claxis_config::schema::find(claxis_config::ConfigFile::Edit, "General.history_depth")
        .unwrap();
    claxis_config::generate::write_value(tmp.dir(), key, "256").unwrap();

    let text = std::fs::read_to_string(tmp.dir().join("edit.toml")).unwrap();
    assert!(text.contains("history_depth = 256"), "{text}");
    assert!(text.contains(key.comment), "комментарий пропал: {text}");
    assert!(text.contains("max_history_depth"), "соседний ключ пропал");
}

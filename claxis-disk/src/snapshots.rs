//! Хранилище снапшотов на диске.
//!
//! Снапшоты одного файла живут кольцом: хранится ровно `keep` штук, и когда
//! приходит новый, самый старый вытесняется. Это делается прямо в базе одной
//! транзакцией, поэтому на диске никогда не бывает больше, чем сказано.
//!
//! Несколько процессов могут работать с одной базой одновременно: SQLite
//! это допускает, и каждый видит чужие записи при следующем чтении.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::path::StorePaths;

/// Один снапшот документа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Какой файл это снапшот.
    pub file: String,
    /// Порядковый номер: чем больше, тем свежее.
    pub seq: u64,
    /// Текст документа на момент снятия.
    pub text: Vec<u8>,
    /// История правок, чтобы отменять было что после перезапуска.
    pub history: Vec<u8>,
}

/// Хранилище снапшотов.
///
/// Снапшоты одного файла живут кольцом: хранится ровно `keep` штук, и когда
/// приходит новый, самый старый вытесняется. Это делается прямо в базе одной
/// транзакцией, поэтому на диске никогда не бывает больше, чем сказано в
/// конфиге.
///
/// Несколько процессов могут работать с одной базой одновременно: SQLite
/// допускает это, и каждый видит чужие записи при следующем чтении.
#[derive(Debug)]
pub struct SnapshotStore {
    conn: Connection,
    /// Куда лежит база. Нужно, чтобы открыть ещё одно подключение.
    path: std::path::PathBuf,
    /// Сколько снапшотов одного файла хранить.
    keep: u8,
}

impl SnapshotStore {
    /// Открыть хранилище, создав базу при первом запуске.
    pub fn open(paths: &StorePaths, keep: u8) -> Result<Self> {
        paths.ensure_dir()?;
        let conn = Connection::open(paths.database()).map_err(|e| Error::Open {
            path: paths.database().display().to_string(),
            reason: e.to_string(),
        })?;
        // WAL позволяет читать и писать одновременно, в том числе из другого
        // процесса, и переживает падение при записи без повреждения базы.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(query)?;
        // Иностранные ключи и синхронность из коробки, вторая нужна для того,
        // чтобы записанный снапшот действительно пережил выход из редактора.
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(query)?;
        let store = Self {
            conn,
            path: paths.database(),
            keep,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Ещё одно подключение к той же базе.
    ///
    /// Буфер живёт дольше сессии, которая его открыла, и не должен её держать.
    /// SQLite это позволяет: подключений может быть сколько угодно.
    pub fn reopen(&self) -> Result<Self> {
        Self::open(
            &StorePaths::new(self.path.parent().unwrap_or(std::path::Path::new("."))),
            self.keep,
        )
    }

    /// Сколько снапшотов одного файла хранится.
    pub fn keep(&self) -> u8 {
        self.keep
    }

    /// Сменить размер кольца.
    ///
    /// Меняется сразу, без перезапуска редактора: это просто число, вокруг
    /// которого крутится выборка. Старые лишние снапшоты не трогаются — они
    /// вытеснятся при следующей записи.
    pub fn set_keep(&mut self, keep: u8) {
        self.keep = keep;
    }

    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS snapshots (
                    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
                    file    TEXT NOT NULL,
                    text    BLOB NOT NULL,
                    history BLOB NOT NULL DEFAULT x''
                 );

                 CREATE INDEX IF NOT EXISTS snapshots_file ON snapshots(file, seq);
                 CREATE TABLE IF NOT EXISTS last_good (
                    name TEXT PRIMARY KEY,
                    body TEXT NOT NULL
                 );",
            )
            .map_err(query)?;
        // Колонка истории появляется после таблицы: на старой базе таблица уже
        // есть, на новой её только что создали, и `ALTER` добавит недостающее.
        self.add_history_column()
    }

    /// Сохранить последний корректный конфиг.
    ///
    /// Он лежит в кеше, а не рядом с настоящим конфигом: пользовательский файл
    /// может быть поломан, а работать надо. Отсюда редактор запускается, если
    /// текущий конфиг не загрузился целиком.
    pub fn save_last_good(&mut self, name: &str, body: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO last_good (name, body) VALUES (?1, ?2)
                 ON CONFLICT(name) DO UPDATE SET body = excluded.body",
                (name, body),
            )
            .map_err(query)?;
        Ok(())
    }

    /// Забрать последний корректный конфиг.
    pub fn last_good(&self, name: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body FROM last_good WHERE name = ?1")
            .map_err(query)?;
        let mut rows = stmt.query([name]).map_err(query)?;
        match rows.next().map_err(query)? {
            Some(row) => Ok(Some(row.get(0).map_err(query)?)),
            None => Ok(None),
        }
    }

    /// Забыть сохранённый конфиг.
    pub fn forget_last_good(&mut self, name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM last_good WHERE name = ?1", [name])
            .map_err(query)?;
        Ok(())
    }

    /// Положить снапшот. Если для этого файла уже есть `keep` штук, самый
    /// старый вытесняется.
    ///
    /// Всё в одной транзакции: либо снапшот записан и старый удалён, либо не
    /// сделано ничего.
    pub fn put(&mut self, file: &Path, text: &[u8], history: &[u8]) -> Result<u64> {
        let file = file.display().to_string();
        let tx = self.conn.transaction().map_err(query)?;
        tx.execute(
            "INSERT INTO snapshots (file, text, history) VALUES (?1, ?2, ?3)",
            params![file, text, history],
        )
        .map_err(query)?;

        // Кольцо: оставляем ровно keep самых свежих для этого файла.
        let stale: Vec<i64> = {
            let mut stmt = tx
                .prepare(
                    "SELECT seq FROM snapshots WHERE file = ?1
                     ORDER BY seq DESC LIMIT -1 OFFSET ?2",
                )
                .map_err(query)?;
            stmt.query_map(params![file, self.keep], |r| r.get(0))
                .map_err(query)?
                .collect::<std::result::Result<Vec<i64>, _>>()
                .map_err(|e| Error::Query {
                    reason: e.to_string(),
                })?
        };
        for seq in stale {
            tx.execute("DELETE FROM snapshots WHERE seq = ?1", params![seq])
                .map_err(query)?;
        }

        let last: i64 = tx
            .query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .map_err(query)?;
        tx.commit().map_err(query)?;
        Ok(last as u64)
    }

    /// Добавить колонку истории в базу, созданную до её появления.
    ///
    /// Миграция идёт на месте: старая база лежит у пользователя, и терять её
    /// из-за нового столбца нельзя. Проверка нужна, потому что `ALTER` падает
    /// на уже существующей колонке, а выполняется он на каждом запуске.
    fn add_history_column(&self) -> Result<()> {
        let sql = "ALTER TABLE snapshots ADD COLUMN history BLOB NOT NULL DEFAULT x''";
        match self.conn.execute_batch(sql) {
            Ok(()) => Ok(()),
            // Колонка уже есть: база не старше миграции, всё в порядке.
            Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
            Err(e) => Err(Error::Query {
                reason: e.to_string(),
            }),
        }
    }

    /// Самый свежий снапшот файла.
    pub fn latest(&self, file: &Path) -> Result<Option<Snapshot>> {
        let file = file.display().to_string();
        self.conn
            .query_row(
                "SELECT seq, text, history FROM snapshots WHERE file = ?1 ORDER BY seq DESC LIMIT 1",
                params![file],
                |r| {
                    Ok(Snapshot {
                        file: file.clone(),
                        seq: r.get::<_, i64>(0)? as u64,
                        text: r.get(1)?,
                        history: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(query)
    }

    /// Снапшот по номеру.
    pub fn get(&self, file: &Path, seq: u64) -> Result<Option<Snapshot>> {
        let file = file.display().to_string();
        self.conn
            .query_row(
                "SELECT seq, text, history FROM snapshots WHERE file = ?1 AND seq = ?2",
                params![file, seq as i64],
                |r| {
                    Ok(Snapshot {
                        file: file.clone(),
                        seq: r.get::<_, i64>(0)? as u64,
                        text: r.get(1)?,
                        history: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(query)
    }

    /// Все снапшоты файла, свежие первыми.
    pub fn list(&self, file: &Path) -> Result<Vec<Snapshot>> {
        let file = file.display().to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT seq, text, history FROM snapshots WHERE file = ?1 ORDER BY seq DESC")
            .map_err(query)?;
        let rows = stmt
            .query_map(params![file], |r| {
                Ok(Snapshot {
                    file: file.clone(),
                    seq: r.get::<_, i64>(0)? as u64,
                    text: r.get(1)?,
                    history: r.get(2)?,
                })
            })
            .map_err(query)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Query {
                reason: e.to_string(),
            })?);
        }
        Ok(out)
    }

    /// Сколько снапшотов хранится у файла.
    pub fn count(&self, file: &Path) -> Result<u32> {
        let file = file.display().to_string();
        let n: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM snapshots WHERE file = ?1",
                params![file],
                |r| r.get(0),
            )
            .map_err(query)?;
        Ok(n as u32)
    }

    /// Убрать все снапшоты файла.
    pub fn forget(&self, file: &Path) -> Result<()> {
        let file = file.display().to_string();
        self.conn
            .execute("DELETE FROM snapshots WHERE file = ?1", params![file])
            .map_err(query)?;
        Ok(())
    }

    /// Сколько всего снапшотов во всех файлах.
    pub fn total(&self) -> Result<u32> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0))
            .map_err(query)?;
        Ok(n as u32)
    }

    /// Сколько база занимает на диске, байт.
    pub fn size_on_disk(&self) -> Result<u64> {
        let page: i64 = self
            .conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(query)?;
        let pages: i64 = self
            .conn
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(query)?;
        Ok((page * pages) as u64)
    }
}

fn query(e: rusqlite::Error) -> Error {
    Error::Query {
        reason: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Отдельный временный каталог под каждый тест: базы не делятся.
    struct Temp {
        dir: std::path::PathBuf,
    }

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-disk-store-{name}"));
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

    fn file(name: &str) -> std::path::PathBuf {
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

    #[test]
    fn keep_is_reported_by_the_store() {
        let temp = Temp::new("keep-report");
        let store = temp.store(5);
        assert_eq!(store.keep(), 5);
    }

    #[test]
    fn a_new_keep_applies_to_the_next_write() {
        // Размер кольца меняется на ходу: следующая запись уже учитывает новое
        // число, а лишние старые снапшоты вытесняются этой же записью.
        let temp = Temp::new("keep-change");
        let target = file("a.rs");
        let mut store = temp.store(5);
        for i in 0..5 {
            store.put(&target, format!("{i}").as_bytes(), b"").unwrap();
        }
        assert_eq!(store.count(&target).unwrap(), 5);

        store.set_keep(2);
        assert_eq!(store.keep(), 2);
        store.put(&target, "ещё".as_bytes(), b"").unwrap();

        assert_eq!(store.count(&target).unwrap(), 2);
        let list = store.list(&target).unwrap();
        assert_eq!(list[0].text, "ещё".as_bytes());
        assert_eq!(list[1].text, "4".as_bytes());
    }

    #[test]
    fn a_grown_keep_does_not_resurrect_old_snapshots() {
        // Увеличение кольца не воскрешает вытесненное: на диске его уже нет.
        let temp = Temp::new("keep-grow");
        let target = file("a.rs");
        let mut store = temp.store(1);
        store.put(&target, "первый".as_bytes(), b"").unwrap();
        store.put(&target, "второй".as_bytes(), b"").unwrap();

        store.set_keep(10);
        store.put(&target, "третий".as_bytes(), b"").unwrap();

        let list = store.list(&target).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            store.latest(&target).unwrap().unwrap().text,
            "третий".as_bytes()
        );
    }

    #[test]
    fn keep_zero_after_writing_clears_the_file() {
        // Перевод хранилища в режим «только память» не оставляет старого на диске.
        let temp = Temp::new("keep-zero-later");
        let target = file("a.rs");
        let mut store = temp.store(3);
        store.put(&target, "старое".as_bytes(), b"").unwrap();

        store.set_keep(0);
        store.put(&target, "новое".as_bytes(), b"").unwrap();

        assert_eq!(store.count(&target).unwrap(), 0);
        assert_eq!(store.total().unwrap(), 0);
    }

    #[test]
    fn reopen_returns_an_independent_connection_to_the_same_base() {
        // Нужно сессии, которая живёт дольше той, что открыла буфер. Данные и
        // размер кольца должны быть теми же.
        let temp = Temp::new("reopen-fn");
        let target = file("a.rs");
        let mut store = temp.store(4);
        store.put(&target, "первый".as_bytes(), b"").unwrap();

        let mut second = store.reopen().unwrap();
        assert_eq!(second.keep(), 4);
        assert_eq!(
            second.latest(&target).unwrap().unwrap().text,
            "первый".as_bytes()
        );

        // Запись во втором подключении видна первому: база одна и та же.
        second.put(&target, "второй".as_bytes(), b"").unwrap();
        assert_eq!(store.count(&target).unwrap(), 2);
    }

    #[test]
    fn keep_is_not_kept_in_the_base_but_taken_from_the_config() {
        // Размер кольца — настройка, а не часть данных: он приходит из конфига при
        // каждом открытии, и в базе не хранится.
        let temp = Temp::new("keep-persist");
        let target = file("a.rs");
        {
            let mut store = temp.store(1);
            store.put(&target, "первый".as_bytes(), b"").unwrap();
            store.set_keep(3);
            store.put(&target, "второй".as_bytes(), b"").unwrap();
        }
        let mut store = temp.store(1);
        assert_eq!(store.count(&target).unwrap(), 2);
        store.put(&target, "третий".as_bytes(), b"").unwrap();
        assert_eq!(store.count(&target).unwrap(), 1);
        assert_eq!(
            store.latest(&target).unwrap().unwrap().text,
            "третий".as_bytes()
        );
    }

    #[test]
    fn history_survives_the_round_trip() {
        // Без истории после перезапуска отменять было бы нечего.
        let temp = Temp::new("history-round");
        let target = file("a.rs");
        let history: Vec<u8> = (0..1000u32).map(|i| (i % 253) as u8).collect();
        let mut store = temp.store(3);
        store.put(&target, "текст".as_bytes(), &history).unwrap();

        let snapshot = store.latest(&target).unwrap().unwrap();
        assert_eq!(snapshot.history, history);
        assert_eq!(snapshot.text, "текст".as_bytes());
        assert_eq!(snapshot.file, target.display().to_string());
    }

    #[test]
    fn history_is_empty_when_nothing_was_passed() {
        let temp = Temp::new("history-empty");
        let target = file("a.rs");
        let mut store = temp.store(3);
        store.put(&target, "текст".as_bytes(), b"").unwrap();
        assert!(store.latest(&target).unwrap().unwrap().history.is_empty());
    }

    #[test]
    fn an_old_base_without_history_still_opens() {
        // Миграция на месте: у человека уже лежит база без колонки истории, и
        // терять её из-за новой версии нельзя.
        let temp = Temp::new("migrate");
        let paths = temp.paths();
        paths.ensure_dir().unwrap();
        {
            let conn = rusqlite::Connection::open(paths.database()).unwrap();
            conn.execute_batch(
                "CREATE TABLE snapshots (
                     seq     INTEGER PRIMARY KEY AUTOINCREMENT,
                     file    TEXT NOT NULL,
                     text    BLOB NOT NULL
                 );
                 CREATE TABLE last_good (name TEXT PRIMARY KEY, body TEXT NOT NULL);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO snapshots (file, text) VALUES (?1, ?2)",
                rusqlite::params![
                    file("старый.rs").display().to_string(),
                    "давний текст".as_bytes()
                ],
            )
            .unwrap();
        }

        let store = temp.store(3);
        let snapshot = store
            .latest(&file("старый.rs"))
            .unwrap()
            .expect("старый снапшот должен найтись");
        assert_eq!(snapshot.text, "давний текст".as_bytes());
        assert!(snapshot.history.is_empty());

        // И новая запись в старую базу пишется, то есть колонка на месте.
        let mut store = store;
        store
            .put(
                &file("старый.rs"),
                "новый текст".as_bytes(),
                "история".as_bytes(),
            )
            .unwrap();
        let fresh = store.latest(&file("старый.rs")).unwrap().unwrap();
        assert_eq!(fresh.text, "новый текст".as_bytes());
        assert_eq!(fresh.history, "история".as_bytes());
    }

    #[test]
    fn opening_twice_on_one_database_does_not_duplicate_the_migration() {
        // `ALTER` падает на уже существующей колонке, а выполняется на каждом
        // запуске: второе открытие не имеет права закончиться ошибкой.
        let temp = Temp::new("migrate-twice");
        let _ = temp.store(3);
        let _ = temp.store(3);
        let _ = temp.store(3);
    }

    #[test]
    fn total_counts_every_file_together() {
        let temp = Temp::new("total");
        let mut store = temp.store(3);
        for name in ["a.rs", "b.rs", "c.rs"] {
            store.put(&file(name), "текст".as_bytes(), b"").unwrap();
            store.put(&file(name), "текст".as_bytes(), b"").unwrap();
        }
        assert_eq!(store.total().unwrap(), 6);
        assert_eq!(store.count(&file("a.rs")).unwrap(), 2);
        assert_eq!(store.count(&file("нет.rs")).unwrap(), 0);
    }

    #[test]
    fn total_is_zero_on_a_fresh_base() {
        let temp = Temp::new("total-empty");
        assert_eq!(temp.store(3).total().unwrap(), 0);
    }

    #[test]
    fn size_on_disk_grows_with_the_data() {
        // Реальная проверка: база с данными тяжелее пустой.
        let temp = Temp::new("size-grow");
        let mut store = temp.store(3);
        let empty = store.size_on_disk().unwrap();
        let target = file("big.rs");
        let blob = vec![b'x'; 2_000_000];
        store.put(&target, &blob, b"").unwrap();
        let full = store.size_on_disk().unwrap();
        assert!(
            full > empty,
            "после записи {empty} байт стало {full} — размер не отражает данные"
        );
    }
}

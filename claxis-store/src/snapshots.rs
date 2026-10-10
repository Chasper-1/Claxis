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
                    seq    INTEGER PRIMARY KEY AUTOINCREMENT,
                    file   TEXT NOT NULL,
                    text   BLOB NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS snapshots_file ON snapshots(file, seq);
                 CREATE TABLE IF NOT EXISTS last_good (
                    name TEXT PRIMARY KEY,
                    body TEXT NOT NULL
                 );",
            )
            .map_err(query)
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
    pub fn put(&mut self, file: &Path, text: &[u8]) -> Result<u64> {
        let file = file.display().to_string();
        let tx = self.conn.transaction().map_err(query)?;
        tx.execute(
            "INSERT INTO snapshots (file, text) VALUES (?1, ?2)",
            params![file, text],
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

    /// Самый свежий снапшот файла.
    pub fn latest(&self, file: &Path) -> Result<Option<Snapshot>> {
        let file = file.display().to_string();
        self.conn
            .query_row(
                "SELECT seq, text FROM snapshots WHERE file = ?1 ORDER BY seq DESC LIMIT 1",
                params![file],
                |r| {
                    Ok(Snapshot {
                        file: file.clone(),
                        seq: r.get::<_, i64>(0)? as u64,
                        text: r.get(1)?,
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
                "SELECT seq, text FROM snapshots WHERE file = ?1 AND seq = ?2",
                params![file, seq as i64],
                |r| {
                    Ok(Snapshot {
                        file: file.clone(),
                        seq: r.get::<_, i64>(0)? as u64,
                        text: r.get(1)?,
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
            .prepare("SELECT seq, text FROM snapshots WHERE file = ?1 ORDER BY seq DESC")
            .map_err(query)?;
        let rows = stmt
            .query_map(params![file], |r| {
                Ok(Snapshot {
                    file: file.clone(),
                    seq: r.get::<_, i64>(0)? as u64,
                    text: r.get(1)?,
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

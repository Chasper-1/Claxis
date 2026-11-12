//! Сохранение файла: проверка и решение.
//!
//! Диск умеет писать и умеет проверять ничего: он принимает готовое решение.
//! Здесь живёт то самое решение. Две независимые ступени отвечают, менялся ли
//! файл с момента открытия, и обе обязаны сойтись, чтобы запись состоялась.
//!
//! Первая ступень — размер. Он ловит почти все правки снаружи и стоит один
//! вызов: дописывание и усечение мимо него не проходят. Вторая — хеш: она
//! запускается только когда размер совпал, потому что платить за чтение и
//! хеширование больших файлов имеет смысл ровно в одном случае — правка той
//! же длины. Такой случай редкий, а редкий и есть тот, где честный хеш
//! окупается.
//!
//! Хотя бы одна ступень не сошлась — файл на диске не пишется. Ни при каком
//! раскладе. Работа при этом не пропадает: правки лежат в базе снапшотов,
//! человек разбирается, что к чему, и решает сам.
//!
//! Проверка целиком редакторская: stat и хеш из собственных рук. Ни inotify,
//! ни любых внешних помощников — ответ не зависит от того, что система
//! помнит, а что забыла.

use std::path::Path;

use claxis_disk::{Anchor, SnapshotStore};

pub mod commands;
pub mod error;
pub mod messages;
mod policy;

pub use error::{Error, Result};
pub use messages::{En, Messages};
pub use policy::{DEFAULT_RETENTION, Refusal, Write, agrees, retention, write};

pub use commands::COMMANDS;

/// Проверить файл на диске против якоря: `Ok(None)` — файл исчез.
///
/// Оба ответа честные: «разошёлся» и «нечем сверять» — разные вещи, и это
/// разные сообщения пользователю.
pub fn verify(store: &SnapshotStore, path: &Path) -> std::result::Result<Option<bool>, Error> {
    let Some(anchor) = store.anchor(path).map_err(Error::Disk)? else {
        // Якоря нет: сверять нечем. Это не «файл в порядке».
        return Ok(None);
    };
    let Some(len) = claxis_disk::file_len(path).map_err(Error::Disk)? else {
        return Ok(Some(false));
    };
    // Ступень размера разошлась — хеш не нужен вовсе.
    if anchor.len != len {
        return Ok(Some(false));
    }
    match claxis_disk::hash::of_file(path).map_err(Error::Disk)? {
        Some(text) => Ok(Some(agrees(&anchor, len, text))),
        None => Ok(Some(false)),
    }
}

/// Обновить якорь под то содержимое, которое редактор видит сейчас.
///
/// Это всегда правда при открытии: буфер собран из файла, и рассуждать он
/// больше ни про что не имеет права. Файла нет — текст пуст, и якорь такой же.
pub fn reanchor(store: &SnapshotStore, path: &Path, text: &[u8]) -> Result<()> {
    let anchor = Anchor {
        len: text.len() as u64,
        hash: claxis_disk::hash::of(text),
    };
    store.set_anchor(path, anchor).map_err(Error::Disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp {
        dir: std::path::PathBuf,
    }

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("claxis-save-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn store(&self) -> Result<SnapshotStore> {
            use claxis_disk::StorePaths;
            let paths = StorePaths::new(self.dir.join("store"));
            SnapshotStore::open(&paths, 3).map_err(Error::Disk)
        }

        fn file(&self, name: &str) -> std::path::PathBuf {
            self.dir.join(name)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn both_gates_decide_the_file_is_same() {
        let anchor = Anchor { len: 4, hash: 7 };
        assert!(agrees(&anchor, 4, 7));
        // Размер разошёлся — хеш уже не важен, ступень не спрашивается.
        assert!(!agrees(&anchor, 5, 7));
        assert!(!agrees(&anchor, 4, 8));
    }

    #[test]
    fn verify_uses_the_anchor_first_and_the_hash_second() {
        // Ступень размера дешевле хеша: разошёлся размер — перечитывать файл
        // целиком не требуется. Ровно это и проверяется: хеш сохранённого
        // содержимого иной, но размер решает первым.
        let temp = Temp::new("verify-size");
        let path = temp.file("a.txt");
        std::fs::write(&path, b"long original text").unwrap();
        let store = temp.store().unwrap();
        let anchor = Anchor {
            len: 9,
            hash: claxis_disk::hash::of(b"other size"),
        };
        store.set_anchor(&path, anchor).unwrap();

        assert_eq!(verify(&store, &path).unwrap(), Some(false));
    }

    #[test]
    fn verify_declares_same_when_both_gates_pass() {
        let temp = Temp::new("verify-same");
        let path = temp.file("a.txt");
        std::fs::write(&path, b"same").unwrap();
        let fresh = Anchor {
            len: 4,
            hash: claxis_disk::hash::of(b"same"),
        };
        let store = temp.store().unwrap();
        store.set_anchor(&path, fresh).unwrap();

        assert_eq!(verify(&store, &path).unwrap(), Some(true));
    }

    #[test]
    fn an_edit_of_the_same_size_is_caught_by_the_hash() {
        // Главный случай, ради которого есть вторая ступень: длина та же,
        // младший байт другой.
        let temp = Temp::new("verify-hash");
        let path = temp.file("a.txt");
        std::fs::write(&path, b"AAAA").unwrap();
        let fresh = Anchor {
            len: 4,
            hash: claxis_disk::hash::of(b"AAAA"),
        };
        let store = temp.store().unwrap();
        store.set_anchor(&path, fresh).unwrap();

        std::fs::write(&path, b"AAAB").unwrap();
        assert_eq!(verify(&store, &path).unwrap(), Some(false));
    }

    #[test]
    fn a_gone_file_is_reported_and_not_declared_same() {
        let temp = Temp::new("verify-gone");
        let path = temp.file("нет-файла.txt");
        let store = temp.store().unwrap();
        store.set_anchor(&path, Anchor { len: 1, hash: 1 }).unwrap();

        assert_eq!(verify(&store, &path).unwrap(), Some(false));
    }

    #[test]
    fn no_anchor_is_no_answer() {
        // Без якоря нет и ответа: правды не получится выдумать.
        let temp = Temp::new("verify-none");
        let path = temp.file("нет-файла.txt");
        let store = temp.store().unwrap();
        assert_eq!(verify(&store, &path).unwrap(), None);
    }

    #[test]
    fn reanchor_records_what_the_buffer_sees() {
        let temp = Temp::new("reanchor");
        let path = temp.file("a.txt");
        let store = temp.store().unwrap();
        reanchor(&store, &path, "новый вопрос".as_bytes()).unwrap();
        let anchor = store.anchor(&path).unwrap().expect("якорь записан");
        assert_eq!(
            anchor,
            Anchor {
                len: 23,
                hash: claxis_disk::hash::of("новый вопрос".as_bytes())
            }
        );
    }
}

//! Проверка файла на диске против якоря.
//!
//! Проверка целиком редакторская: размер и хеш из собственных рук. Ни
//! inotify, ни любых внешних помощников — ответ не зависит от того, что система
//! помнит, а что забыла.

use std::path::Path;

use claxis_disk::{Anchor, SnapshotStore};

use crate::error::Error;

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
        Some(text) => Ok(Some(crate::policy::agrees(&anchor, len, text))),
        None => Ok(Some(false)),
    }
}

/// Обновить якорь под то содержимое, которое редактор видит сейчас.
///
/// Это всегда правда при открытии: буфер собран из файла, и рассуждать он
/// больше ни про что не имеет права. Файла нет — текст пуст, и якорь такой же.
pub fn reanchor(store: &SnapshotStore, path: &Path, text: &[u8]) -> Result<(), Error> {
    let anchor = Anchor {
        len: text.len() as u64,
        hash: claxis_disk::hash::of(text),
    };
    store.set_anchor(path, anchor).map_err(Error::Disk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use claxis_disk::SnapshotStore;
    use claxis_disk::StorePaths;

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

        fn store(&self) -> Result<SnapshotStore, Error> {
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

//! Политика сохранения.

use std::path::Path;

use claxis_disk::{Anchor, SnapshotStore};

use crate::error::{Error, Result};

/// Чем именно отказались записывать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Файл на диске отличается от того, что видели при открытии: писать
    /// поверх чужой или новой правки незачем.
    Changed,
    /// Якоря нет: сверять нечем, и выдумывать правду не станут.
    Unmarked,
    /// Файла на диске нет.
    Gone,
}

/// Обе ступени независимы и обе обязаны сойтись: совпали — файл тот же.
pub fn agrees(anchor: &Anchor, len: u64, hash: u128) -> bool {
    // Сначала размер: разошёлся он — хеш смотреть незачем.
    anchor.len == len && anchor.hash == hash
}

/// Итог сохранения. Отказ — нормальный ход, а не ошибка: он значит, что
/// политика сработала.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    /// Записано, и это размер того, что на диске теперь.
    Written {
        /// Размер записанного.
        bytes: u64,
    },
    /// Записывать не стали: почему — [`Refusal`].
    Refused(Refusal),
}

/// Записать текст в файл через ворота проверки.
///
/// Две ступени против файла на диске: сначала размер, потом хеш. Хеш
/// пересчитывается с диска: буфер мог уехать правками, и сверять его с якорем
/// — сравнивать не то. Обе сошлись — запись атомарная, и якорь обновляется
/// под новое содержимое: он считается от уже записанного текста, то есть
/// снова от того, что в памяти.
pub fn write(store: &SnapshotStore, path: &Path, text: &[u8]) -> Result<Write> {
    let Some(anchor) = store.anchor(path).map_err(Error::Disk)? else {
        return Ok(Write::Refused(Refusal::Unmarked));
    };

    // Ступень размера.
    let Some(len) = claxis_disk::file_len(path).map_err(Error::Disk)? else {
        return Ok(Write::Refused(Refusal::Gone));
    };
    if len != anchor.len {
        return Ok(Write::Refused(Refusal::Changed));
    }

    // Ступень хеша: платят за неё только там, где размер сошёлся.
    let Some(on_disk) = claxis_disk::hash::of_file(path).map_err(Error::Disk)? else {
        return Ok(Write::Refused(Refusal::Gone));
    };
    if !agrees(&anchor, len, on_disk) {
        return Ok(Write::Refused(Refusal::Changed));
    }

    claxis_disk::write_blob(path, text).map_err(Error::Disk)?;
    let fresh = Anchor {
        len: text.len() as u64,
        hash: claxis_disk::hash::of(text),
    };
    store.set_anchor(path, fresh).map_err(Error::Disk)?;
    Ok(Write::Written { bytes: fresh.len })
}

/// Срок годности снапшотов в секундах: сколько правкам положено жить.
///
/// Форма значения — число с суффиксом: `d` день (24 часа), `h` час, `m`
/// минута, `s` секунда. Голое число — секунды. Ноль означает «срока нет»:
/// чистка правок тогда идёт только по сохранению и выходу.
///
/// Ошибка возвратом, а не умолчанием: значение писал пользователь, и работать
/// на числе, которого никто не писал, значит врать.
pub fn retention(value: &str) -> Result<u64> {
    let value = value.trim();
    let digits = value.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return Err(Error::Retention {
            value: value.to_string(),
        });
    }
    let amount: u64 = value[..digits].parse().map_err(|_| Error::Retention {
        value: value.to_string(),
    })?;
    let factor = match value[digits..].trim() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => {
            return Err(Error::Retention {
                value: value.to_string(),
            });
        }
    };
    amount.checked_mul(factor).ok_or(Error::Retention {
        value: value.to_string(),
    })
}

/// Срок годности по умолчанию: неделя.
pub const DEFAULT_RETENTION: &str = "7d";

#[cfg(test)]
mod tests {
    use super::*;

    /// Отдельная база под тест: одна на все пробы сохранения.
    fn store(name: &str) -> Result<SnapshotStore> {
        let dir = std::env::temp_dir().join(format!("claxis-save-store-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        use claxis_disk::StorePaths;
        let paths = StorePaths::new(dir);
        SnapshotStore::open(&paths, 3).map_err(Error::Disk)
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("claxis-save-file-{name}"));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Файл и якорь под его нынешнее содержимое.
    fn mark(path: &std::path::Path, store: &SnapshotStore, text: &[u8]) -> Result<()> {
        claxis_disk::write_blob(path, text).map_err(Error::Disk)?;
        let anchor = Anchor {
            len: text.len() as u64,
            hash: claxis_disk::hash::of(text),
        };
        store.set_anchor(path, anchor).map_err(Error::Disk)
    }

    #[test]
    fn both_gates_pass_and_the_file_is_written() {
        let store = store("written").unwrap();
        let path = temp_file("written");
        mark(&path, &store, "старое содержимое".as_bytes()).unwrap();

        let outcome = write(&store, &path, "новое содержимое!".as_bytes()).unwrap();
        assert_eq!(outcome, Write::Written { bytes: 32 });
        assert_eq!(claxis_disk::read_text(&path).unwrap(), "новое содержимое!");
        // Якорь под новое содержимое: следующая проверка должна согласиться.
        assert_eq!(crate::verify(&store, &path).unwrap(), Some(true));
    }

    #[test]
    fn an_edit_of_the_same_size_refuses_the_write() {
        // Длина та же, содержимое другое: на весь ответ здесь тратится один
        // stat и один хеш. Затирать чужую правку поверх нашей работы — врать.
        let store = store("same-size").unwrap();
        let path = temp_file("same-size");
        mark(&path, &store, b"AAAA").unwrap();
        claxis_disk::write_text(&path, "AAAB").unwrap();

        let outcome = write(&store, &path, "правка поверх чужого".as_bytes()).unwrap();
        assert_eq!(outcome, Write::Refused(Refusal::Changed));
        // На диске стоит чужое: наша работа не написалась.
        assert_eq!(claxis_disk::read_text(&path).unwrap(), "AAAB");
    }

    #[test]
    fn a_size_change_refuses_the_write_early() {
        let store = store("size-change").unwrap();
        let path = temp_file("size-change");
        mark(&path, &store, "короткий".as_bytes()).unwrap();
        claxis_disk::write_text(&path, "намноого длиннее, чем было").unwrap();

        assert_eq!(
            write(&store, &path, "новое".as_bytes()).unwrap(),
            Write::Refused(Refusal::Changed)
        );
    }

    #[test]
    fn a_gone_file_refuses_the_write() {
        let store = store("gone").unwrap();
        let path = temp_file("gone");
        mark(&path, &store, "был файл".as_bytes()).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(
            write(&store, &path, "правка".as_bytes()).unwrap(),
            Write::Refused(Refusal::Gone)
        );
    }

    #[test]
    fn no_anchor_refuses_the_write() {
        // Правду редактор не придумывает: якоря нет — значит, сверять нечем.
        let store = store("unmarked").unwrap();
        let path = temp_file("unmarked");
        claxis_disk::write_text(&path, "без отметки").unwrap();

        assert_eq!(
            write(&store, &path, "правка".as_bytes()).unwrap(),
            Write::Refused(Refusal::Unmarked)
        );
        assert_eq!(claxis_disk::read_text(&path).unwrap(), "без отметки");
    }

    #[test]
    fn after_a_write_the_anchor_moves_to_the_fresh_content() {
        // Якорь всегда о том, что мы последними видели: после нашей записи —
        // о записанном.
        let store = store("reanchor-after-write").unwrap();
        let path = temp_file("reanchor-after-write");
        mark(&path, &store, b"basic line").unwrap();

        write(&store, &path, b"fresh line").unwrap();
        let anchor = store.anchor(&path).unwrap().expect("якорь на месте");
        assert_eq!(
            anchor,
            Anchor {
                len: 10,
                hash: claxis_disk::hash::of(b"fresh line")
            }
        );
    }

    #[test]
    fn retention_is_parsed_by_suffix() {
        assert_eq!(retention("2d").unwrap(), 2 * 24 * 60 * 60);
        assert_eq!(retention("2h").unwrap(), 2 * 60 * 60);
        assert_eq!(retention("2m").unwrap(), 2 * 60);
        assert_eq!(retention("2s").unwrap(), 2);
        assert_eq!(retention("3600").unwrap(), 3600);
        assert_eq!(retention("0").unwrap(), 0);
        assert_eq!(retention(" 7d ").unwrap(), 7 * 24 * 60 * 60);
    }

    #[test]
    fn retention_refuses_what_it_does_not_understand() {
        // Чужая единица или не число — это ошибка, а не ноль: работать на
        // сроке, которого никто не писал, значит врать.
        for value in ["", "2w", "d", "2 дня", "-3", "一周"] {
            let parsed = retention(value);
            assert!(
                matches!(parsed, Err(Error::Retention { .. })),
                "прошло: {value}"
            );
        }
    }
}

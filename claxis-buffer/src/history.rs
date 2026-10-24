//! Сохранённая ветка правок поверх файла на диске.
//!
//! Формат свой, rope его владеет целиком. Снапшот нужен затем, чтобы после
//! перезапуска можно было вернуть несохранённую работу: исходный текст
//! читается из файла, а сюда уходят только изменённые куски и записи правок.
//!
//! Начало файла — четыре байта магии и версия. Магия нужна не для красоты:
//! прежний буфер писал в ту же колонку свой формат, и такой блоб должен
//! распознаваться как чужой и отбрасываться, а не разбираться как родной.
//!
//! ```text
//! "CLRP" u8:версия | u32:длина файла | u64:отпечаток файла
//!      | u32:длина хвоста | хвост | u32:сколько записей | запись...
//! запись: u8:вид | u32:якорь | u32:голова | u32:текст
//! ```

use crate::Result;
use crate::document::{Document, Record};

/// Размер заголовка: магия, версия, длина файла, отпечаток и длина хвоста.
const HEADER: usize = 4 + 1 + 4 + 8 + 4;

/// Магия формата rope.
pub const MAGIC: [u8; 4] = *b"CLRP";

/// Версия формата rope.
///
/// Третья: блоб больше не несёт исходный текст, только хвост правок и
/// отпечаток файла. Старый блоб отвергается целиком — файл при этом остаётся
/// на месте, теряется только история.
pub const VERSION: u8 = 3;

/// Байт записи вставки.
const INSERT: u8 = 0;

/// Байт записи удаления.
const DELETE: u8 = 1;

/// Байт записи замены.
const REPLACE: u8 = 2;

/// Размер одной записи: вид, якорь, голова, текст, вырезанное и его длина.
///
/// Записи одинаковой длины даже там, где часть полей не используется: сквозной
/// размер разбирается без сдвига и ветвлений, а лишние нули стоят копейки.
const RECORD: usize = 21;

/// Размер одной записи истории в байтах.
///
/// Открыто наружу: тесты и хранилище должны знать, сколько байт занимает
/// запись, иначе проверка формата будет гадать.
pub const RECORD_SIZE: usize = RECORD;

/// Сохранённая ветка правок.
///
/// Обёртка над байтами: пока это просто содержимое снапшота, но с собственным
/// именем у него появятся поля, и менять их можно будет внутри крейта.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    bytes: Vec<u8>,
}

impl History {
    /// Пустая история: документа как будто не трогали.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Байты истории — то, что уходит в хранилище.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Есть ли что восстанавливать.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Тот ли это файл, поверх которого снималась история.
    ///
    /// Исходный текст в блобе не лежит: его читают с диска. Совпасть должны
    /// длина и отпечаток — иначе правка, сделанная снаружи, молча уедет под
    /// следующей записью.
    pub fn matches_base(&self, base: &[u8]) -> Result<bool> {
        let header = Header::parse(&self.bytes)?;
        Ok(header.base_len == base.len() && header.base_sum == sum(base))
    }

    /// Принять байты из хранилища и проверить, что блоб наш и целый.
    ///
    /// В базе лежит то, что положил прошлый запуск: прежний буфер писал
    /// туда свой формат, блоб мог оборваться. Разбор проверяет всё, до
    /// последней записи, и отвечает заранее: годный блоб или нет.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let header = Header::parse(bytes)?;
        let mut offset = header.records_at;
        for _ in 0..header.count {
            let chunk = bytes
                .get(offset..offset + RECORD)
                .ok_or(crate::Error::TruncatedHistory)?;
            let read =
                |i: usize| u32::from_le_bytes([chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]]);
            let kind = chunk[0];
            if kind != INSERT && kind != DELETE && kind != REPLACE {
                return Err(crate::Error::UnknownRecordKind { kind });
            }
            // Смещения пришли из базы, доверять им нельзя: из-за вывернутого
            // диапазона вычитание переполнилось бы, а кусок за границами
            // буфера уронил бы редактор вместе с несохранённым текстом.
            let (anchor, head, text_offset) = (read(1), read(5), read(9));
            if head < anchor {
                return Err(crate::Error::InvalidRecordRange { anchor, head });
            }
            fits(text_offset, head - anchor, header.buf_len)?;
            if kind == REPLACE {
                fits(read(13), read(17), header.buf_len)?;
            }
            offset += RECORD;
        }
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    /// Собрать историю из документа.
    ///
    /// Уезжает только хвост буфера: вставленное и вырезанное. Исходный текст
    /// остаётся в файле, и записи ссылаются на хвост по тем же смещениям,
    /// какие были в памяти.
    pub fn capture(doc: &Document) -> Self {
        let records = doc.path();
        let base_len = doc.file_len() as usize;
        let buffer = doc.buffer();
        let tail = &buffer[base_len..];
        // Размер блоба известен заранее: заголовок, хвост и по
        // `RECORD_SIZE` байту на запись. Берём сразу, чтобы не расти по кускам.
        let mut bytes = Vec::with_capacity(HEADER + tail.len() + 4 + records.len() * RECORD_SIZE);
        bytes.extend_from_slice(&MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&doc.file_len().to_le_bytes());
        bytes.extend_from_slice(&sum(&buffer[..base_len]).to_le_bytes());
        bytes.extend_from_slice(&(tail.len() as u32).to_le_bytes());
        bytes.extend_from_slice(tail);
        bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
        for record in records {
            let (removed, removed_len) = record.removed().unwrap_or((0, 0));
            bytes.push(match record {
                Record::Insert { .. } => INSERT,
                Record::Delete { .. } => DELETE,
                Record::Replace { .. } => REPLACE,
            });
            bytes.extend_from_slice(&record.anchor().to_le_bytes());
            bytes.extend_from_slice(&record.head().to_le_bytes());
            bytes.extend_from_slice(&record.text_offset().to_le_bytes());
            bytes.extend_from_slice(&removed.to_le_bytes());
            bytes.extend_from_slice(&removed_len.to_le_bytes());
        }
        Self { bytes }
    }

    /// Разобрать сохранённое состояние и собрать документ.
    ///
    /// Исходный текст приходит готовым: он лежит в файле, а не в истории.
    /// Хвост правок дописывается к нему, и только потом применяются записи.
    ///
    /// Глубину истории задаёт вызывающий: в снимке её нет, а восстановленный
    /// документ должен продолжать работу с той же настройкой, а не с
    /// умолчанием.
    pub fn restore(&self, base: &[u8], depth: u32) -> Result<Document> {
        let header = Header::parse(&self.bytes)?;
        let got = sum(base);
        if header.base_len != base.len() || header.base_sum != got {
            return Err(crate::Error::HistoryBaseMismatch {
                expected: header.base_sum,
                got,
            });
        }
        let bytes = self.bytes.as_slice();
        let tail = bytes
            .get(header.tail_at..header.tail_at + header.tail_len)
            .ok_or(crate::Error::TruncatedHistory)?;
        let mut buffer = Vec::with_capacity(base.len().saturating_add(tail.len()));
        buffer.extend_from_slice(base);
        buffer.extend_from_slice(tail);
        let mut offset = header.records_at;
        let count = header.count;

        let mut doc = Document::from_buffer_with_depth(buffer, base.len(), depth);
        for _ in 0..count {
            let chunk = bytes
                .get(offset..offset + RECORD)
                .ok_or(crate::Error::TruncatedHistory)?;
            let read =
                |i: usize| u32::from_le_bytes([chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]]);
            let record = match chunk[0] {
                INSERT => Record::Insert {
                    anchor: read(1),
                    head: read(5),
                    text_offset: read(9),
                },
                DELETE => Record::Delete {
                    anchor: read(1),
                    head: read(5),
                    text_offset: read(9),
                },
                _ => Record::Replace {
                    anchor: read(1),
                    head: read(5),
                    text_offset: read(9),
                    removed_offset: read(13),
                    removed_len: read(17),
                },
            };
            offset += RECORD;
            // Смещения записей — позиции в документе, а не в буфере. Битую
            // запись дерево приняло бы молча: вставка за концом читала бы
            // чужой кусок, а удаление ушло бы за границу. Проверяем их до
            // того, как тронуть дерево.
            let doc_len = doc.len();
            let anchor = record.anchor();
            if anchor > doc_len {
                return Err(crate::Error::OutOfBounds {
                    anchor,
                    len: 0,
                    doc_len,
                });
            }
            let head = record.head();
            let span = match record {
                Record::Insert { .. } => 0,
                Record::Delete { .. } => head - anchor,
                Record::Replace { removed_len, .. } => removed_len,
            };
            if anchor as u64 + span as u64 > doc_len as u64 {
                return Err(crate::Error::OutOfBounds {
                    anchor,
                    len: span,
                    doc_len,
                });
            }
            doc.apply(record);
        }
        Ok(doc)
    }
}
/// Помещается ли кусок `len` байт, начиная со смещения `offset`, в буфер `buf_len`.
fn fits(offset: u32, len: u32, buf_len: usize) -> Result<()> {
    let end = offset as u64 + len as u64;
    if end > buf_len as u64 {
        return Err(crate::Error::HistoryOutOfBounds { offset, len });
    }
    Ok(())
}

/// Отпечаток исходного текста: FNV-1a.
///
/// Хранится вместо самого текста. Совпадение отпечатка и длины не доказывает
/// побайтового равенства, но ловит правку снаружи без гигабайтов в базе.
fn sum(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// Разобранный заголовок: где что лежит.
struct Header {
    /// Длина исходного текста.
    base_len: usize,
    /// Отпечаток исходного текста.
    base_sum: u64,
    /// Длина хвоста правок.
    tail_len: usize,
    /// Смещение, где начинается хвост.
    tail_at: usize,
    /// Смещение, где начинаются записи.
    records_at: usize,
    /// Сколько записей.
    count: usize,
    /// Длина всего буфера: файл и хвост.
    buf_len: usize,
}

impl Header {
    fn parse(b: &[u8]) -> Result<Header> {
        let read_u32 = |offset: usize| -> Option<u32> {
            let c = b.get(offset..offset + 4)?;
            Some(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        };
        let read_u64 = |offset: usize| -> Option<u64> {
            let c = b.get(offset..offset + 8)?;
            Some(u64::from_le_bytes([
                c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7],
            ]))
        };
        if b.get(..4) != Some(MAGIC.as_slice()) {
            return Err(crate::Error::NotOurHistory);
        }
        let version = *b.get(4).ok_or(crate::Error::TruncatedHistory)?;
        if version != VERSION {
            return Err(crate::Error::UnknownHistoryVersion { version });
        }
        let base_len = read_u32(5).ok_or(crate::Error::TruncatedHistory)? as usize;
        let base_sum = read_u64(9).ok_or(crate::Error::TruncatedHistory)?;
        let tail_len = read_u32(17).ok_or(crate::Error::TruncatedHistory)? as usize;
        let tail_at = HEADER;
        let buf_len = base_len
            .checked_add(tail_len)
            .ok_or(crate::Error::TruncatedHistory)?;
        let records_at = tail_at
            .checked_add(tail_len)
            .ok_or(crate::Error::TruncatedHistory)?;
        let count = read_u32(records_at).ok_or(crate::Error::TruncatedHistory)? as usize;
        let needed = records_at
            .checked_add(4)
            .and_then(|offset| offset.checked_add(count.checked_mul(RECORD)?))
            .ok_or(crate::Error::TruncatedHistory)?;
        if b.len() < needed {
            return Err(crate::Error::TruncatedHistory);
        }
        Ok(Header {
            base_len,
            base_sum,
            tail_len,
            tail_at,
            records_at: records_at + 4,
            count,
            buf_len,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::document::Document;

    fn d(text: &str) -> Document {
        Document::new(text.as_bytes()).expect("valid document")
    }

    #[test]
    fn a_foreign_blob_is_not_read_as_our_history() {
        // Прежний буфер писал в ту же колонку свой формат. Такой блоб обязан быть
        // распознан как чужой, а не разобран как родной.
        let mut old = Vec::new();
        old.extend_from_slice(b"BUFH");
        old.extend_from_slice(&[0u8; 32]);
        assert_eq!(History::decode(&old).unwrap_err(), Error::NotOurHistory);
        assert!(History::decode(&[]).is_err());
    }

    #[test]
    fn a_history_from_another_version_is_refused() {
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        let history = History::capture(&doc);
        let mut bytes = history.as_bytes().to_vec();
        bytes[4] = 99;
        assert_eq!(
            History::decode(&bytes).unwrap_err(),
            Error::UnknownHistoryVersion { version: 99 }
        );
    }

    #[test]
    fn a_truncated_history_is_refused_not_panicked() {
        // Снапшот может прийти битым: обрыв, обрезка, чужой мусор. Всё это —
        // ошибка с текстом, а не паника.
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        doc.delete(0, 1).unwrap();
        let history = History::capture(&doc);
        let bytes = history.as_bytes().to_vec();
        for cut in 1..bytes.len() {
            let err = History::decode(&bytes[..cut]);
            assert!(err.is_err(), "обрыв на {cut} байте разобрался как целый");
        }
    }

    #[test]
    fn an_unknown_record_kind_is_refused() {
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        let mut bytes = History::capture(&doc).as_bytes().to_vec();
        let last = bytes.len() - RECORD_SIZE;
        bytes[last] = 7;
        assert_eq!(
            History::decode(&bytes).unwrap_err(),
            Error::UnknownRecordKind { kind: 7 }
        );
    }

    #[test]
    fn a_record_with_a_backwards_range_is_refused() {
        // Из-за `head - anchor` переполнилось бы вычитание, а это паника.
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        let mut bytes = History::capture(&doc).as_bytes().to_vec();
        let last = bytes.len() - RECORD_SIZE;
        // anchor и head стоят подряд, начиная с первого поля записи.
        bytes[last + 1..last + 5].copy_from_slice(&9u32.to_le_bytes());
        bytes[last + 5..last + 9].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            History::decode(&bytes).unwrap_err(),
            Error::InvalidRecordRange { anchor: 9, head: 2 }
        );
    }

    #[test]
    fn a_record_pointing_outside_the_buffer_is_refused() {
        // Куски за границами буфера не существует. Раньше отмена удаления резала
        // буфер по такому смещению и роняла редактор вместе с текстом.
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        doc.delete(0, 1).unwrap();
        let mut bytes = History::capture(&doc).as_bytes().to_vec();
        let last = bytes.len() - RECORD_SIZE;
        bytes[last + 9..last + 13].copy_from_slice(&9_000u32.to_le_bytes());
        assert_eq!(
            History::decode(&bytes).unwrap_err(),
            Error::HistoryOutOfBounds {
                offset: 9_000,
                len: 1
            }
        );
    }

    #[test]
    fn a_history_from_another_file_is_refused() {
        // Исходный текст в блобе не лежит. Если файл на диске успели
        // поменять, восстанавливать поверх него нельзя: правка снаружи
        // станет невидимой и уедет под следующим сохранением.
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        let history = History::capture(&doc);
        assert!(history.matches_base(b"abc").unwrap());
        assert!(!history.matches_base(b"abd").unwrap());
        assert!(!history.matches_base(b"abcd").unwrap());
        assert_eq!(
            history.restore(b"abd", crate::DEFAULT_DEPTH).unwrap_err(),
            Error::HistoryBaseMismatch {
                expected: sum(b"abc"),
                got: sum(b"abd"),
            }
        );
    }

    #[test]
    fn a_replace_removed_range_outside_the_buffer_is_refused() {
        // У записи замены два диапазона: вставленное и вырезанное. Проверять
        // надо оба: вырезанный кусок тоже читается из буфера при отмене.
        let mut doc = d("abcdef");
        doc.replace(1, 2, b"XY").unwrap();
        let mut bytes = History::capture(&doc).as_bytes().to_vec();
        let last = bytes.len() - RECORD_SIZE;
        assert_eq!(bytes[last], REPLACE, "ожидалась запись замены");
        bytes[last + 13..last + 17].copy_from_slice(&9_000u32.to_le_bytes());
        let removed_len = u32::from_le_bytes([
            bytes[last + 17],
            bytes[last + 18],
            bytes[last + 19],
            bytes[last + 20],
        ]);
        assert_eq!(
            History::decode(&bytes).unwrap_err(),
            Error::HistoryOutOfBounds {
                offset: 9_000,
                len: removed_len
            }
        );
    }

    #[test]
    fn the_base_is_the_original_file_after_edits() {
        // Отпечаток базы — это то, чем файл был, когда бралась история. После
        // правок он обязан остаться прежним, а не стать текущим текстом.
        let mut doc = d("original");
        for i in 0..10u32 {
            doc.insert(i % (doc.len() + 1), b"x").unwrap();
        }
        assert_ne!(doc.read(), b"original", "правки должны были изменить текст");
        let history = History::capture(&doc);
        assert!(
            history.matches_base(b"original").unwrap(),
            "база — исходный файл, а не текущий текст"
        );
        assert!(!history.matches_base(&doc.read()).unwrap());
    }

    #[test]
    fn a_captured_history_survives_a_round_trip() {
        // Обратная сторона проверок: хорошая история разбирается и
        // восстанавливается.
        let mut doc = d("abc");
        doc.insert(3, b"XY").unwrap();
        doc.replace(0, 2, b"Z").unwrap();
        doc.delete(1, 1).unwrap();
        let history = History::capture(&doc);
        let mut restored = History::decode(history.as_bytes())
            .unwrap()
            .restore(b"abc", 7)
            .unwrap();
        assert_eq!(restored.read(), doc.read());
        assert_eq!(
            restored.history_depth(),
            7,
            "восстановление обязано вернуть заданную глубину истории"
        );
        restored.undo().unwrap().expect("отменять есть что");
        assert_eq!(restored.read(), b"ZcXY");
    }

    #[test]
    fn a_big_file_does_not_travel_into_the_history() {
        // В блоб уходит только хвост правок. Исходный текст уже лежит в файле,
        // и копировать его в базу на каждый снимок незачем.
        let base = vec![b'a'; 1_000_000];
        let mut doc = Document::new(&base).unwrap();
        doc.insert(500_000, b"xy").unwrap();
        let history = History::capture(&doc);
        assert!(
            history.as_bytes().len() < 1000,
            "история заняла {} байт",
            history.as_bytes().len()
        );
        assert!(history.matches_base(&base).unwrap());
        assert_eq!(
            history
                .restore(&base, crate::DEFAULT_DEPTH)
                .unwrap()
                .read()
                .len(),
            base.len() + 2
        );
    }

    #[test]
    fn the_history_survives_the_database_round_trip() {
        // Ровно то, что делает редактор: снапшот уходит в базу байтами и
        // приходит байтами же.
        let mut doc = d("начало");
        for i in 0..50u32 {
            if i % 3 == 0 {
                doc.insert(i % (doc.len() + 1), b"pq").unwrap();
            } else if !doc.is_empty() {
                let at = i % doc.len();
                doc.delete(at, 1).unwrap();
            }
        }
        let text = doc.read();
        let stored = History::capture(&doc).as_bytes().to_vec();

        let back = History::decode(&stored)
            .expect("блоб из базы разбирается")
            .restore("начало".as_bytes(), crate::DEFAULT_DEPTH)
            .expect("документ собирается");
        assert_eq!(back.read(), text);
        assert_eq!(back.path().len(), doc.path().len());
    }

    #[test]
    fn an_untouched_document_carries_no_records() {
        // Пустая история — обычное состояние, а не поломка.
        let doc = d("текст без правок");
        let history = History::capture(&doc);
        // Заголовок, пустой хвост и счётчик записей.
        let bytes = history.as_bytes();
        let at = HEADER;
        assert_eq!(
            &bytes[at..at + 4],
            &0u32.to_le_bytes(),
            "в пустой истории не должно быть ни одной записи, а в блобе: {bytes:?}"
        );
        assert!(doc.path().is_empty());
        assert_eq!(
            bytes.len(),
            at + 4,
            "кроме заголовка и нулевого счётчика ничего нет"
        );
        let mut back = history
            .restore("текст без правок".as_bytes(), crate::DEFAULT_DEPTH)
            .expect("пустая история разбирается");
        assert_eq!(back.read(), doc.read());
        assert!(back.path().is_empty());
        assert!(back.undo().unwrap().is_none(), "отменять нечего");
    }
}

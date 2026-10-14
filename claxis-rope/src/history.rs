//! Сохранённое состояние документа: текстовая база и ветка правок.
//!
//! Формат свой, rope его владеет целиком. Снапшот нужен затем, чтобы после
//! перезапуска отменять было что: без истории документ верный, но работа,
//! сделанная после прошлого запуска, отменяться не будет.
//!
//! Начало файла — четыре байта магии и версия. Магия нужна не для красоты:
//! прежний буфер писал в ту же колонку свой формат, и такой блоб должен
//! распознаваться как чужой и отбрасываться, а не разбираться как родной.
//!
//! ```text
//! "CLRP" u8:версия | u32:длина файла | u32:длина буфера | буфер
//!            | u32:сколько записей | запись...
//! запись:    u8:вид | u32:якорь | u32:голова | u32:текст
//! ```

use crate::Result;
use crate::document::{Document, Record};

/// Магия формата rope.
pub const MAGIC: [u8; 4] = *b"CLRP";

/// Версия формата rope.
///
/// Вторая: запись замены выросла на два поля, и разбор первой версии такую
/// запись не поймёт. Старый блоб отвергается целиком — текст из базы при этом
/// остаётся верным, теряется только история.
pub const VERSION: u8 = 2;

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

    /// Принять байты из хранилища и проверить, что блоб наш и целый.
    ///
    /// В базе лежит то, что положил прошлый запуск: прежний буфер писал
    /// туда свой формат, блоб мог оборваться. Разбор проверяет всё, до
    /// последней записи, и отвечает заранее: годный блоб или нет.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let header = Header::parse(bytes)?;
        for kind in bytes[header.records_at..].iter().step_by(RECORD) {
            let kind = *kind;
            if kind != INSERT && kind != DELETE && kind != REPLACE {
                return Err(crate::Error::UnknownRecordKind { kind });
            }
        }
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    /// Собрать историю из документа.
    ///
    /// Уезжает весь текстовый буфер, а не только текст файла: записи ссылаются
    /// на хвост, куда уезжает вставленное и вырезанное, и эти смещения должны
    /// остаться верными после восстановления.
    pub fn capture(doc: &Document) -> Self {
        let records = doc.path();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&doc.file_len().to_le_bytes());
        bytes.extend_from_slice(&(doc.buffer_len() as u32).to_le_bytes());
        bytes.extend_from_slice(doc.buffer());
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
            bytes.extend_from_slice(&record.text().to_le_bytes());
            bytes.extend_from_slice(&removed.to_le_bytes());
            bytes.extend_from_slice(&removed_len.to_le_bytes());
        }
        Self { bytes }
    }

    /// Разобрать сохранённое состояние и собрать документ.
    ///
    /// Текст документа берётся из истории: он и есть состояние. Отдельный
    /// `text` не нужен — он записан рядом, тем же файлом.
    pub fn restore(&self) -> Result<Document> {
        let header = Header::parse(&self.bytes)?;
        let b = self.bytes.as_slice();
        let file_len = header.file_len;
        let buf_len = header.buf_len;
        let buffer = b
            .get(header.buffer_at..header.buffer_at + buf_len)
            .ok_or(crate::Error::TruncatedHistory)?;
        let mut offset = header.records_at;
        let count = header.count;

        let mut doc = Document::from_buffer(buffer.to_vec(), file_len);
        for _ in 0..count {
            let chunk = b
                .get(offset..offset + RECORD)
                .ok_or(crate::Error::TruncatedHistory)?;
            let read =
                |i: usize| u32::from_le_bytes([chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]]);
            let record = match chunk[0] {
                INSERT => Record::Insert {
                    anchor: read(1),
                    head: read(5),
                    text: read(9),
                },
                DELETE => Record::Delete {
                    anchor: read(1),
                    head: read(5),
                    text: read(9),
                },
                _ => Record::Replace {
                    anchor: read(1),
                    head: read(5),
                    text: read(9),
                    removed: read(13),
                    removed_len: read(17),
                },
            };
            offset += RECORD;
            doc.apply(record);
        }
        Ok(doc)
    }
}
/// Разобранный заголовок: где что лежит.
struct Header {
    /// Длина текста файла.
    file_len: usize,
    /// Длина текстового буфера.
    buf_len: usize,
    /// Смещение, где начинается текст.
    buffer_at: usize,
    /// Смещение, где начинаются записи.
    records_at: usize,
    /// Сколько записей.
    count: usize,
}

impl Header {
    fn parse(b: &[u8]) -> Result<Header> {
        let u32_at = |at: usize| -> Option<u32> {
            let c = b.get(at..at + 4)?;
            Some(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        };
        if b.get(..4) != Some(MAGIC.as_slice()) {
            return Err(crate::Error::NotOurHistory);
        }
        let version = *b.get(4).ok_or(crate::Error::TruncatedHistory)?;
        if version != VERSION {
            return Err(crate::Error::UnknownHistoryVersion { version });
        }
        let file_len = u32_at(5).ok_or(crate::Error::TruncatedHistory)? as usize;
        let buf_len = u32_at(9).ok_or(crate::Error::TruncatedHistory)? as usize;
        let buffer_at: usize = 13;
        let records_at = buffer_at
            .checked_add(buf_len)
            .ok_or(crate::Error::TruncatedHistory)?;
        let count = u32_at(records_at).ok_or(crate::Error::TruncatedHistory)? as usize;
        let needed = records_at
            .checked_add(4)
            .and_then(|at| at.checked_add(count.checked_mul(RECORD)?))
            .ok_or(crate::Error::TruncatedHistory)?;
        if b.len() < needed {
            return Err(crate::Error::TruncatedHistory);
        }
        Ok(Header {
            file_len,
            buf_len,
            buffer_at,
            records_at: records_at + 4,
            count,
        })
    }
}

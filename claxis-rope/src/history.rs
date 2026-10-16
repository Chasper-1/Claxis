//! История правок, которая переживает снапшот.
//!
//! Снапшот отдаёт наружу не только текст, но и историю: иначе после
//! перезапуска отменять было бы нечего, хотя правки ещё на экране.
//!
//! Формат — плотный, без выравнивания: одна запись 13 байт. Размер значения
//! неважен, важен порядок: записи идут от самой старой к самой новой.

/// Запись истории в виде байт.
pub type Bytes = Vec<u8>;

/// История: стеки отмены и повтора плюс тексты вставок.
///
/// Хранится уже готовым к записи, потому что буфер не должен знать, куда его
/// положить: это дело приёмника снапшота.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// Текст, к которому применяются записи: `Original` на момент снапшота.
    ///
    /// Без него историю применять не к чему: записи описывают переход
    /// **от** этого текста. Сохранять надо состояние целиком, а не только
    /// результат, — иначе после перезапуска текст верный, а отменять
    /// нечего.
    base: Bytes,
    undo: Bytes,
    redo: Bytes,
    added: Bytes,
}

/// Запись вставки: где вставлено, где в Added лежит текст.
const INSERT: u8 = 0;
/// Запись удаления: что вырезано.
const DELETE: u8 = 1;
/// Длина одной записи: метка и три `u32`.
pub const RECORD_LEN: usize = 13;
/// Заголовок: четыре длины, каждая `u32`.
const HEADER_LEN: usize = 16;

/// Пустая история: отменять нечего.
pub fn empty() -> History {
    History::default()
}

impl History {
    /// Собрать историю из базы, стеков и текстов.
    pub fn new(base: Bytes, undo: Bytes, redo: Bytes, added: Bytes) -> Self {
        Self {
            base,
            undo,
            redo,
            added,
        }
    }

    /// Текст, к которому применяются записи.
    pub fn base(&self) -> &[u8] {
        &self.base
    }

    /// Записи отмены.
    pub fn undo(&self) -> &[u8] {
        &self.undo
    }

    /// Записи повтора.
    pub fn redo(&self) -> &[u8] {
        &self.redo
    }

    /// Тексты вставок.
    pub fn added(&self) -> &[u8] {
        &self.added
    }

    /// История пустая: отменять нечего.
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// Запаковать в байты: три длины, потом сами данные.
    pub fn encode(&self) -> Bytes {
        let mut out = Vec::with_capacity(
            HEADER_LEN + self.base.len() + self.undo.len() + self.redo.len() + self.added.len(),
        );
        for part in [&self.base, &self.undo, &self.redo, &self.added] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    /// Распаковать. Мусор на входе — пустая история, а не паника:
    /// потеря истории не должна ломать открытие документа.
    pub fn decode(bytes: &[u8]) -> Self {
        let mut cursor = Cursor::new(bytes);
        let (Some(base), Some(undo), Some(redo), Some(added)) = (
            cursor.take_part(),
            cursor.take_part(),
            cursor.take_part(),
            cursor.take_part(),
        ) else {
            return History::default();
        };
        History::new(base, undo, redo, added)
    }
}

/// Чтение частей по длине, объявленной в заголовке.
struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Длина и содержимое следующей части. `None`, если не хватило байт.
    fn take_part(&mut self) -> Option<Bytes> {
        let head = self.bytes.get(..4)?;
        let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let rest = self.bytes.get(4..)?;
        let (part, tail) = rest.split_at_checked(len)?;
        self.bytes = tail;
        Some(part.to_vec())
    }
}

/// Записать одну запись истории в конец буфера.
pub fn write_record(out: &mut Bytes, tag: u8, anchor: u32, head: u32, text: u32) {
    out.push(tag);
    out.extend_from_slice(&anchor.to_le_bytes());
    out.extend_from_slice(&head.to_le_bytes());
    out.extend_from_slice(&text.to_le_bytes());
}

/// Разобрать одну запись истории.
pub fn read_record(bytes: &[u8]) -> Option<(u8, u32, u32, u32)> {
    let part = bytes.get(..RECORD_LEN)?;
    Some((
        part[0],
        u32::from_le_bytes([part[1], part[2], part[3], part[4]]),
        u32::from_le_bytes([part[5], part[6], part[7], part[8]]),
        u32::from_le_bytes([part[9], part[10], part[11], part[12]]),
    ))
}

/// Метка записи удаления.
pub const DELETE_TAG: u8 = DELETE;
/// Метка записи вставки.
pub const INSERT_TAG: u8 = INSERT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_travels_with_the_history() {
        let history = History::new(b"abc".to_vec(), Bytes::new(), Bytes::new(), b"XY".to_vec());
        assert_eq!(History::decode(&history.encode()).base(), b"abc");
    }

    #[test]
    fn a_record_is_thirteen_bytes() {
        let mut out = Bytes::new();
        write_record(&mut out, INSERT_TAG, 1, 5, 9);
        assert_eq!(out.len(), RECORD_LEN);
        assert_eq!(read_record(&out), Some((INSERT_TAG, 1, 5, 9)));
    }

    #[test]
    fn history_survives_the_round_trip() {
        let mut undo = Bytes::new();
        write_record(&mut undo, INSERT_TAG, 3, 8, 0);
        let mut redo = Bytes::new();
        write_record(&mut redo, DELETE_TAG, 0, 4, 0);
        let history = History::new(b"base".to_vec(), undo, redo, b"hello".to_vec());

        let back = History::decode(&history.encode());
        assert_eq!(back, history);
    }

    #[test]
    fn an_empty_history_is_empty() {
        assert!(empty().is_empty());
        assert_eq!(History::decode(&[]), empty());
    }

    #[test]
    fn broken_bytes_give_an_empty_history() {
        // Потеря истории не должна ломать открытие документа.
        assert_eq!(History::decode(&[1, 2, 3]), empty());
        assert_eq!(History::decode(&[255; 32]), empty());
    }

    #[test]
    fn a_truncated_record_does_not_panic() {
        let mut undo = Bytes::new();
        write_record(&mut undo, INSERT_TAG, 1, 2, 3);
        undo.pop();
        assert_eq!(read_record(&undo), None);
    }
}

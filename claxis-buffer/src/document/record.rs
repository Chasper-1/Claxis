/// Запись правки. Операция несётся самой записью.
///
/// Смещения названы тем, чем они являются: `text_offset` — с какого места общего
/// буфера начинается текст этой записи. Это не текст документа и не позиция в нём.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Record {
    /// Вставка: куда, сколько и где в буфере лежит вставленное.
    Insert {
        /// Где в документе.
        anchor: u32,
        /// Где кончается вставка: это её длина.
        head: u32,
        /// С какого места буфера начинается вставленный текст.
        text_offset: u32,
    },
    /// Удаление: что вырезано и где в буфере это лежит.
    Delete {
        /// Где начинается вырезанный диапазон.
        anchor: u32,
        /// Где кончается вырезанный диапазон.
        head: u32,
        /// С какого места буфера начинается вырезанный текст: он вернётся при
        /// отмене.
        text_offset: u32,
    },
    /// Замена диапазона: что вырезано, что вставлено и где оба текста.
    ///
    /// Одна запись, а не две: замена — одно действие пользователя и один
    /// шаг отмены. Разбирать её на удаление и вставку значило бы платить два
    /// раза за перестройку дерева и путать историю.
    Replace {
        /// Где начинается вырезанный диапазон.
        anchor: u32,
        /// Где кончается заменённый диапазон: это длина вставленного.
        head: u32,
        /// С какого места буфера начинается вставленный текст.
        text_offset: u32,
        /// С какого места буфера начинается вырезанный текст: он вернётся при
        /// отмене.
        removed_offset: u32,
        /// Сколько байт было вырезано.
        removed_len: u32,
    },
}

impl Record {
    /// Длина текста самой записи: у вставки и замены — вставленного, у
    /// удаления — вырезанного.
    ///
    /// У замены это только вставленное: вырезанное считается по
    /// [`Record::removed`] и в `len` не входит — иначе сдвиг `text_offset`
    /// разъехался бы с длиной.
    pub fn len(&self) -> u32 {
        // Один разбор варианта вместо двух: `anchor()` и `head()` — это два
        // отдельных `match` по той же записи.
        match self {
            Record::Insert { anchor, head, .. }
            | Record::Delete { anchor, head, .. }
            | Record::Replace { anchor, head, .. } => anchor.abs_diff(*head),
        }
    }

    /// Несёт ли запись свой текст.
    ///
    /// Замена с пустой вставкой, но непустым `removed_len` — не пустая: текст
    /// вставки у неё пуст, а вырезанное есть. Отменять такую всё равно нужно.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Начало диапазона.
    pub fn anchor(&self) -> u32 {
        match self {
            Record::Insert { anchor, .. }
            | Record::Delete { anchor, .. }
            | Record::Replace { anchor, .. } => *anchor,
        }
    }

    /// Конец диапазона.
    pub fn head(&self) -> u32 {
        match self {
            Record::Insert { head, .. }
            | Record::Delete { head, .. }
            | Record::Replace { head, .. } => *head,
        }
    }

    /// С какого места буфера начинается текст записи.
    pub fn text_offset(&self) -> u32 {
        match self {
            Record::Insert { text_offset, .. } | Record::Delete { text_offset, .. } => *text_offset,
            Record::Replace { text_offset, .. } => *text_offset,
        }
    }

    /// Смещение вырезанного текста, если запись его несёт.
    ///
    /// У вставки и удаления вырезанного текста нет: вставка ничего не
    /// вырезала, а удаление ничего не вставляло.
    pub fn removed(&self) -> Option<(u32, u32)> {
        match self {
            Record::Replace {
                removed_offset,
                removed_len,
                ..
            } => Some((*removed_offset, *removed_len)),
            _ => None,
        }
    }

    /// Вид записи для файла истории.
    pub fn kind(&self) -> Kind {
        match self {
            Record::Insert { .. } => Kind::Insert,
            Record::Delete { .. } => Kind::Delete,
            Record::Replace { .. } => Kind::Replace,
        }
    }
}

/// Вид записи истории.
///
/// Отдельный тип, а не число: что попадёт в файл определяется здесь, и
/// разбор читает то же самое. Перепутать местами нельзя.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Запись вставки.
    Insert,
    /// Запись удаления.
    Delete,
    /// Запись замены.
    Replace,
}

/// Самое позднее место буфера, куда смотрит запись: её собственный текст и
/// вырезанный ею.
pub(super) fn record_end(record: &Record) -> u32 {
    // Один разбор варианта: собственный текст и вырезанный считаются за раз,
    // а не тремя `match` подряд.
    match record {
        Record::Insert {
            anchor,
            head,
            text_offset,
        }
        | Record::Delete {
            anchor,
            head,
            text_offset,
        } => text_offset.saturating_add(anchor.abs_diff(*head)),
        Record::Replace {
            anchor,
            head,
            text_offset,
            removed_offset,
            removed_len,
        } => {
            let inserted = text_offset.saturating_add(anchor.abs_diff(*head));
            inserted.max(removed_offset.saturating_add(*removed_len))
        }
    }
}

/// Вершина истории: запись, родитель, ребёнок и граница буфера.
#[derive(Clone, Copy, Debug)]
pub(super) struct Version {
    pub(super) record: Record,
    pub(super) parent: u32,
    pub(super) child: u32,
    /// Самое позднее место буфера, куда заглядывает эта вершина или любая
    /// выше неё по родителям.
    ///
    /// Считается один раз при добавлении вершины, а обходом ветки не
    /// пересчитывается: иначе граница обрезки буфера стоила бы столько же,
    /// сколько вся история. Разрез зависит только от записей, и у каждой
    /// вершины он известен сразу.
    pub(super) live: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_counts_the_records_own_text() {
        let insert = Record::Insert {
            anchor: 3,
            head: 8,
            text_offset: 100,
        };
        assert_eq!(insert.len(), 5);
        assert!(!insert.is_empty());

        let delete = Record::Delete {
            anchor: 3,
            head: 8,
            text_offset: 100,
        };
        assert_eq!(delete.len(), 5);

        // У замены длина — только вставленное, вырезанное считается отдельно.
        let replace = Record::Replace {
            anchor: 3,
            head: 5,
            text_offset: 100,
            removed_offset: 200,
            removed_len: 9,
        };
        assert_eq!(replace.len(), 2);
        assert!(!replace.is_empty());

        // Замена без вставки: своего текста нет, но вырезанное есть.
        let cut = Record::Replace {
            anchor: 3,
            head: 3,
            text_offset: 0,
            removed_offset: 0,
            removed_len: 4,
        };
        assert_eq!(cut.len(), 0);
        assert!(cut.is_empty());
        assert_eq!(cut.removed(), Some((0, 4)));
    }

    #[test]
    fn len_is_safe_on_a_backwards_range() {
        // Испорченная запись не должна переполнять вычитание.
        let broken = Record::Delete {
            anchor: 8,
            head: 3,
            text_offset: 0,
        };
        assert_eq!(broken.len(), 5);
    }

    #[test]
    fn record_end_reaches_the_farther_text() {
        let insert = Record::Insert {
            anchor: 0,
            head: 4,
            text_offset: 10,
        };
        assert_eq!(record_end(&insert), 14);

        let delete = Record::Delete {
            anchor: 0,
            head: 4,
            text_offset: 10,
        };
        assert_eq!(record_end(&delete), 14);

        // Вырезанный текст может лежать дальше вставленного: берётся дальний.
        let replace = Record::Replace {
            anchor: 0,
            head: 2,
            text_offset: 10,
            removed_offset: 50,
            removed_len: 3,
        };
        assert_eq!(record_end(&replace), 53);

        // Когда вырезанный ближе вставленного, конец задаёт вставленный текст.
        let near = Record::Replace {
            anchor: 0,
            head: 2,
            text_offset: 10,
            removed_offset: 12,
            removed_len: 3,
        };
        assert_eq!(record_end(&near), 15);
    }
}

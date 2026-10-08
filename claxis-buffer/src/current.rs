use crate::arena::{ARENA_CAPACITY, Kind};
use crate::segment::{ADDED, Segment};

/// Сколько сегментов помещается в арену записей.
pub const ARENA_RECORDS: usize = ARENA_CAPACITY / size_of::<crate::arena::Record>();

/// Каждая запись арены добавляет в буфер не больше двух сегментов: вставка
/// разрезает один на три, удаление откладывает края. Плюс один сегмент на
/// исходный текст. Буфер выделяется на это количество один раз и больше не
/// растёт: арена ограничена, значит и сегментов ограничено.
pub const SEG_CAPACITY: usize = 1 + 2 * ARENA_RECORDS;

/// Материализованное состояние документа: последовательность сегментов.
///
/// Сегменты лежат в буфере по порядку документа, поэтому шаг — это индекс
/// плюс или минус один.
///
/// Позиция курсора — байтовая. Чтобы дойти до новой позиции, считается разница
/// между новой и старой: вперёд длины прибавляются, назад вычитаются. Никакого
/// поиска от начала не происходит.
///
/// Память под буфер выделена целиком при создании и дальше не перевыделяется.
/// Читаются сегменты `0..count`. Удалённые сегменты не уничтожаются: они
/// откладываются за живой областью, и отмена возвращает их в чтение. Смещение
/// отложенного — относительное, поэтому переезд отложенной области не ломает
/// сохранённые ссылки.
#[derive(Debug)]
pub struct Current {
    segs: Vec<Segment>,
    /// Сколько сегментов читается.
    count: usize,
    /// Сколько сегментов отложено в хвосте буфера: `segs[parked_from..]`.
    parked: usize,
    /// Начало отложенной области. Значимо только при `parked > 0`.
    parked_from: usize,
    /// Длина документа в байтах.
    len: u32,
    /// Длина исходного текста: с неё начинается пересборка.
    original_len: u32,
    /// Позиция курсора: байт в документе, а также сегмент и смещение в нём.
    cur_pos: u32,
    cur_idx: usize,
    cur_off: u32,
    /// Временные буферы для собираемых последовательностей сегментов. Память
    /// выделена один раз и переиспользуется: буферы не отдаются и не удаляются,
    /// поэтому в горячем пути аллокаций нет.
    scratch: Vec<Segment>,
    /// Что остаётся в чтении при удалении.
    kept: Vec<Segment>,
    /// Что уходит из чтения при удалении.
    removed: Vec<Segment>,
}

impl Default for Current {
    fn default() -> Self {
        Self::from_original(&[])
    }
}

impl Current {
    pub fn from_original(original: &[u8]) -> Self {
        let len = original.len() as u32;
        // Память выделяется один раз на весь срок жизни арены и больше не
        // трогается: `segs` только заполняется.
        let mut segs = Vec::with_capacity(SEG_CAPACITY);
        if len > 0 {
            segs.push(Segment::original(0, len));
        }
        let count = segs.len();
        Self {
            parked: 0,
            parked_from: segs.len(),
            segs,
            count,
            len,
            original_len: len,
            cur_pos: 0,
            cur_idx: 0,
            cur_off: 0,
            scratch: Vec::with_capacity(SEG_CAPACITY),
            kept: Vec::with_capacity(SEG_CAPACITY),
            removed: Vec::with_capacity(SEG_CAPACITY),
        }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Сегмент и смещение внутри него для байта `pos`.
    ///
    /// Идём от курсора на разницу между новой и старой позицией: вперёд индекс
    /// растёт и длины прибавляются, назад индекс убывает и длины вычитаются.
    fn locate(&mut self, pos: u32) -> (usize, u32) {
        debug_assert!(pos <= self.len, "позиция за концом документа");
        let delta = pos as i64 - self.cur_pos as i64;
        let (idx, off) = if delta >= 0 {
            self.walk_forward(self.cur_idx, self.cur_off, delta as u32)
        } else {
            self.walk_back(self.cur_idx, self.cur_off, (-delta) as u32)
        };
        debug_assert!(
            self.count == 0 || idx < self.count,
            "курсор ушёл за живые сегменты"
        );
        self.cur_pos = pos;
        self.cur_idx = idx;
        self.cur_off = off;
        (idx, off)
    }

    fn walk_forward(&self, mut idx: usize, mut off: u32, mut left: u32) -> (usize, u32) {
        while left > 0 {
            let room = self.segs[idx].len() - off;
            if left < room {
                off += left;
                break;
            }
            off += room;
            left -= room;
            if left == 0 {
                break;
            }
            idx += 1;
            debug_assert!(idx < self.count, "позиция за концом документа");
            off = 0;
        }
        (idx, off)
    }

    fn walk_back(&self, mut idx: usize, mut off: u32, mut left: u32) -> (usize, u32) {
        while left > 0 {
            if off >= left {
                off -= left;
                break;
            }
            left -= off;
            debug_assert!(idx > 0, "позиция до начала документа");
            idx -= 1;
            off = self.segs[idx].len();
        }
        (idx, off)
    }

    /// Ставит курсор в известную точку: сегмент, смещение, позиция.
    fn place_cursor(&mut self, idx: usize, off: u32, pos: u32) {
        self.cur_idx = idx;
        self.cur_off = off;
        self.cur_pos = pos;
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segs[..self.count]
    }

    pub fn read(&self, original: &[u8], added: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len as usize);
        self.read_into(&mut out, original, added);
        out
    }

    pub fn read_into(&self, out: &mut Vec<u8>, original: &[u8], added: &[u8]) {
        let bases = [original.as_ptr(), added.as_ptr()];
        // Сегменты подряд, читаем подряд: промахов кэша нет.
        for seg in &self.segs[..self.count] {
            debug_assert!(seg.len() > 0, "нулевой сегмент: {seg:?}");
            let base = unsafe { *bases.get_unchecked((seg.src == ADDED) as usize) };
            let bytes = unsafe {
                std::slice::from_raw_parts(base.add(seg.off as usize), seg.len() as usize)
            };
            out.extend_from_slice(bytes);
        }
    }

    /// Вставка `len` байт из растущего буфера, начиная с `text_off`.
    pub(crate) fn apply_insert(&mut self, pos: u32, text_off: u32, len: u32) {
        debug_assert!(len > 0, "нулевая вставка");
        self.insert_segments(pos, &[Segment::added(text_off, len)]);
        self.set_len(self.len as i64 + len as i64);
    }

    /// Возврат отложенных сегментов в чтение: отмена удаления.
    pub(crate) fn restore_deleted(&mut self, pos: u32, parked: u32, len: u32) {
        let restored = self.take_parked(parked, len);
        let total: u32 = restored.iter().map(|seg| seg.len()).sum();
        debug_assert_eq!(total, len, "отложенные сегменты не покрывают диапазон");
        self.insert_segments(pos, &restored);
        self.set_len(self.len as i64 + len as i64);
    }

    /// Исключение диапазона из чтения. Байты остаются на месте: отложенные
    /// сегменты описывают ровно удалённый диапазон и ни байтом больше.
    /// Возвращает их относительное смещение в отложенной области.
    pub(crate) fn apply_delete(&mut self, pos: u32, len: u32) -> u32 {
        debug_assert!(len > 0, "нулевое удаление");
        debug_assert!(pos + len <= self.len, "удаление за концом документа");
        let (first, start_off) = self.locate(pos);
        let (last, end_off) = self.locate(pos + len);
        let head = self.segs[first];
        let tail = self.segs[last];

        // Что остаётся в чтении: неразрезанные края диапазона.
        self.kept.clear();
        if start_off > 0 {
            self.kept.push(Segment::new(head.src, head.off, start_off));
        }
        if end_off < tail.len() {
            self.kept.push(Segment::new(
                tail.src,
                tail.off + end_off,
                tail.len() - end_off,
            ));
        }
        let put_len = self.kept.len();

        // Что уходит из чтения: ровно удалённые байты.
        self.removed.clear();
        if first == last {
            self.removed.push(Segment::new(
                head.src,
                head.off + start_off,
                end_off - start_off,
            ));
        } else {
            if start_off < head.len() {
                self.removed.push(Segment::new(
                    head.src,
                    head.off + start_off,
                    head.len() - start_off,
                ));
            }
            for i in first + 1..last {
                self.removed.push(self.segs[i]);
            }
            if end_off > 0 {
                self.removed.push(Segment::new(tail.src, tail.off, end_off));
            }
        }
        let parked = park(
            &mut self.segs,
            &mut self.parked,
            &mut self.parked_from,
            &self.removed,
        );

        let taken = last - first + 1;
        splice(
            &mut self.segs,
            &mut self.count,
            self.parked,
            &mut self.parked_from,
            first,
            taken,
            &self.kept,
        );
        self.set_len(self.len as i64 - len as i64);
        // Позиция `pos` — это начало того, что осталось. Если не осталось ничего,
        // это конец предыдущего сегмента, а если и его нет — начало документа.
        match (self.count, put_len, start_off) {
            (0, _, _) => self.place_cursor(0, 0, pos),
            (_, 0, _) if first > 0 => {
                let idx = first - 1;
                self.place_cursor(idx, self.segs[idx].len(), pos);
            }
            (_, _, 0) => self.place_cursor(first, 0, pos),
            _ => self.place_cursor(first, start_off, pos),
        }
        parked
    }

    /// Ставит готовую последовательность сегментов на позицию `pos`.
    fn insert_segments(&mut self, pos: u32, segs: &[Segment]) {
        let end = pos + segs.iter().map(|seg| seg.len()).sum::<u32>();
        let (idx, mut off) = self.locate(pos);
        if idx < self.count {
            off = off.min(self.segs[idx].len());
        }

        // Позиция может попасть ровно на конец сегмента — это обычное разбиение,
        // хвост при этом пустой и в последовательность не входит.
        self.scratch.clear();
        let mut taken = 0usize;
        if off > 0 {
            taken = 1;
            let host = self.segs[idx];
            self.scratch.push(Segment::new(host.src, host.off, off));
        }
        let inserted = segs.len();
        self.scratch.extend_from_slice(segs);
        if taken == 1 {
            let host = self.segs[idx];
            if off < host.len() {
                self.scratch
                    .push(Segment::new(host.src, host.off + off, host.len() - off));
            }
        }
        splice(
            &mut self.segs,
            &mut self.count,
            self.parked,
            &mut self.parked_from,
            idx,
            taken,
            &self.scratch,
        );
        // Курсор встаёт на конец вставленного: конец документа при вставке в
        // конец, сама позиция — иначе.
        if self.count == 0 {
            self.place_cursor(0, 0, end);
        } else {
            let last = idx + taken + inserted - 1;
            self.place_cursor(last, self.segs[last].len(), end);
        }
    }

    /// Забирает отложенные сегменты общей длиной `bytes`, начиная со смещения.
    fn take_parked(&self, parked: u32, bytes: u32) -> Vec<Segment> {
        let mut at = self.parked_from + parked as usize;
        let mut out = Vec::new();
        let mut total = 0u32;
        while total < bytes {
            debug_assert!(at < self.segs.len(), "отложенные сегменты кончились");
            let seg = self.segs[at];
            total += seg.len();
            out.push(seg);
            at += 1;
        }
        out
    }

    /// Пересборка из исходного текста и активной истории, по порядку.
    pub(crate) fn rebuild<'a>(&mut self, active: impl Iterator<Item = (Kind, u32, u32, u32)>) {
        self.segs.clear();
        self.count = 0;
        self.parked = 0;
        if self.original_len > 0 {
            self.segs.push(Segment::original(0, self.original_len));
            self.count = 1;
        }
        self.parked_from = self.segs.len();
        self.len = self.original_len;
        self.place_cursor(0, 0, 0);
        for (kind, pos, len, data) in active {
            match kind {
                Kind::Insert => self.apply_insert(pos, data, len),
                // Смещение отложенных сегментов записанной правки здесь не
                // годится: пересборка откладывает заново и получает своё.
                Kind::Delete => {
                    self.apply_delete(pos, len);
                }
            }
        }
    }

    fn set_len(&mut self, next: i64) {
        debug_assert!(next >= 0, "длина документа стала отрицательной");
        debug_assert!(next <= u32::MAX as i64, "документ длиннее 4 ГиБ");
        self.len = next as u32;
    }
}

/// Живая область помещается до отложенной, иначе отложенная область переезжает
/// целиком. Относительные смещения переживают переезд.
fn ensure_live_room(
    segs: &mut Vec<Segment>,
    parked: usize,
    parked_from: &mut usize,
    needed: usize,
) {
    if needed > segs.len() {
        debug_assert!(
            needed <= SEG_CAPACITY,
            "буфер сегментов переполнен: {needed} при ёмкости {SEG_CAPACITY}"
        );
        segs.resize(needed, Segment::original(0, 0));
    }
    if parked == 0 || needed <= *parked_from {
        return;
    }
    let size = parked;
    let fit = needed + size;
    debug_assert!(
        fit <= SEG_CAPACITY,
        "буфер сегментов переполнен: {fit} при ёмкости {SEG_CAPACITY}"
    );
    segs.resize(fit, Segment::original(0, 0));
    segs.copy_within(*parked_from..*parked_from + size, needed);
    *parked_from = needed;
}

/// Заменяет `taken` сегментов с индекса `at` на `put`.
fn splice(
    segs: &mut Vec<Segment>,
    count: &mut usize,
    parked: usize,
    parked_from: &mut usize,
    at: usize,
    taken: usize,
    put: &[Segment],
) {
    debug_assert!(at + taken <= *count, "диапазон за пределами документа");
    let live = *count;
    let grown = live - taken + put.len();
    // Нужная длина — `grown`, а не `at + grown`: старший индекс, который
    // трогает splice, равен `grown - 1`. Передача с лишним `at` заставляла
    // resize заполнять нулями область за живыми сегментами на каждой правке.
    ensure_live_room(segs, parked, parked_from, grown);
    let src = at + taken;
    let dst = at + put.len();
    if src < live {
        segs.copy_within(src..live, dst);
    }
    segs[at..dst].copy_from_slice(put);
    *count = grown;
}

/// Кладёт сегменты в отложенную область. Смещение относительное к началу
/// отложенной, поэтому переезд области не ломает сохранённую ссылку.
fn park(
    segs: &mut Vec<Segment>,
    parked: &mut usize,
    parked_from: &mut usize,
    items: &[Segment],
) -> u32 {
    let n = items.len();
    let dest = segs.len();
    debug_assert!(
        dest + n <= SEG_CAPACITY,
        "буфер сегментов переполнен: {} при ёмкости {SEG_CAPACITY}",
        dest + n
    );
    segs.resize(dest + n, Segment::original(0, 0));
    segs[dest..dest + n].copy_from_slice(items);
    let rel = if *parked == 0 { 0 } else { dest - *parked_from };
    if *parked == 0 {
        *parked_from = dest;
    }
    *parked += n;
    rel as u32
}

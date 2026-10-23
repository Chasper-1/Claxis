//! Карта строк: где в документе переводы строк.
//!
//! Хранит позиции переводов строк по порядку. По ней за O(log n) находится
//! номер строки для позиции и позиция для номера строки — это нужно для
//! перехода «на строку N» и для относительных номеров строк в статус-строке.
//!
//! Живёт отдельно от документа: документ знает про байты и ничего не знает
//! про строки. Сюда приходят только дельты — «здесь вставилось столько-то
//! байт», «здесь вырезалось столько-то» — и карта пересчитывает себя сама.
//! Никакой связи с устройством документа тут нет и не должно быть.

/// Вид перевода строки.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreakKind {
    /// `\n`.
    Lf,
    /// `\r` без следующего `\n`.
    Cr,
    /// `\r\n` — один перевод.
    CrLf,
}

impl BreakKind {
    /// Длина перевода в байтах.
    const fn len(self) -> u32 {
        match self {
            Self::CrLf => 2,
            Self::Lf | Self::Cr => 1,
        }
    }
}

/// Один перевод строки: начало и вид.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineBreak {
    start: u32,
    kind: BreakKind,
}

impl LineBreak {
    /// Позиция сразу после перевода.
    const fn end(self) -> u32 {
        self.start + self.kind.len()
    }

    /// Разложить перевод на отдельные байты `\r` и `\n`.
    fn atoms(self, out: &mut Vec<(u32, bool)>) {
        match self.kind {
            BreakKind::Cr => out.push((self.start, true)),
            BreakKind::Lf => out.push((self.start, false)),
            BreakKind::CrLf => {
                out.push((self.start, true));
                out.push((self.start + 1, false));
            }
        }
    }
}

/// Карта переводов строк.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMap {
    /// Переводы строк по возрастанию.
    ///
    /// Список, а не дерево: сдвиг при вставке в начало стоит копирования
    /// хвоста, но это единицы мегабайт на правку.
    breaks: Vec<LineBreak>,
    /// Длина документа в байтах: нужна, чтобы не пускать вставку за конец.
    len: u32,
}

impl LineMap {
    /// Пустая карта: документ без единого перевода строки.
    pub fn new() -> Self {
        Self::default()
    }

    /// Построить карту по тексту с нуля.
    pub fn build(text: &[u8]) -> Self {
        let mut map = Self::new();
        map.reset(text);
        map
    }

    /// Пересобрать карту по тексту: после открытия файла или снапшота.
    pub fn reset(&mut self, text: &[u8]) {
        let atoms = scan_atoms(text, 0);
        self.breaks = group_atoms(&atoms);
        self.len = text.len() as u32;
    }

    /// Вставка текста на позицию `pos`.
    ///
    /// Позиция за концом документа считается концом: карта не хранит переводы
    /// строк за пределами текста. Вставка внутри `\r\n` разрезает его, а
    /// `\r` и `\n`, оказавшиеся рядом, снова склеиваются в один перевод.
    pub fn insert(&mut self, pos: u32, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let pos = pos.min(self.len);
        let added = data.len() as u32;

        // Захватываем соседние переводы: с ними вставленный кусок может
        // склеиться в `\r\n` или разрезать существующий `\r\n`.
        let first = self
            .breaks
            .partition_point(|b| b.end() <= pos)
            .saturating_sub(1);
        let last = (self.breaks.partition_point(|b| b.start < pos) + 1).min(self.breaks.len());

        let mut atoms = Vec::new();
        let mut shifted = Vec::new();
        for line_break in &self.breaks[first..last] {
            let mut part = Vec::new();
            line_break.atoms(&mut part);
            for (at, cr) in part {
                if at < pos {
                    atoms.push((at, cr));
                } else {
                    shifted.push((at + added, cr));
                }
            }
        }
        atoms.extend(scan_atoms(data, pos));
        atoms.extend(shifted);

        let grouped = group_atoms(&atoms);
        let grouped_len = grouped.len();
        self.breaks.splice(first..last, grouped);
        // Всё, что стоит дальше окна, сдвигается на длину вставки.
        let tail = first + grouped_len;
        self.breaks[tail..]
            .iter_mut()
            .for_each(|line_break| line_break.start += added);
        self.len += added;
    }

    /// Удаление диапазона `from..to`.
    ///
    /// Переводы внутри диапазона пропадают, оставшиеся `\r` и `\n` могут
    /// снова стать одним `\r\n`, а всё, что стояло дальше, сдвигается влево.
    pub fn delete(&mut self, from: u32, to: u32) {
        let from = from.min(self.len);
        let to = to.min(self.len);
        if to <= from {
            return;
        }
        let removed = to - from;

        let first = self
            .breaks
            .partition_point(|b| b.end() <= from)
            .saturating_sub(1);
        let last = (self.breaks.partition_point(|b| b.start < to) + 1).min(self.breaks.len());

        let mut atoms = Vec::new();
        for line_break in &self.breaks[first..last] {
            let mut part = Vec::new();
            line_break.atoms(&mut part);
            for (at, cr) in part {
                if at < from {
                    atoms.push((at, cr));
                } else if at >= to {
                    atoms.push((at - removed, cr));
                }
            }
        }

        let grouped = group_atoms(&atoms);
        let grouped_len = grouped.len();
        self.breaks.splice(first..last, grouped);
        let tail = first + grouped_len;
        self.breaks[tail..]
            .iter_mut()
            .for_each(|line_break| line_break.start -= removed);
        self.len -= removed;
    }

    /// Сколько строк в документе: перевод строки плюс последняя строка.
    pub fn line_count(&self) -> u32 {
        self.breaks.len() as u32 + 1
    }

    /// Переводов строк без последней строки.
    pub fn break_count(&self) -> usize {
        self.breaks.len()
    }

    /// Номер строки для позиции `pos`, начиная с нуля.
    ///
    /// Позиция после перевода строки принадлежит следующей строке: нажатие
    /// Enter в конце строки переводит курсор на новую строку, и он должен быть
    /// на ней, а не на предыдущей.
    pub fn line_of(&self, pos: u32) -> u32 {
        self.breaks.partition_point(|b| b.end() <= pos) as u32
    }

    /// Позиция, с которой начинается строка `line`, начиная с нуля.
    ///
    /// Строка `k + 1` начинается сразу после перевода строки `k`. За пределами
    /// документа — конец документа: прыгать выше конца бессмысленно.
    pub fn offset_of_line(&self, line: u32, doc_len: u32) -> u32 {
        if line == 0 {
            return 0;
        }
        match self.breaks.get(line as usize - 1) {
            Some(line_break) => line_break.end(),
            None => doc_len,
        }
    }

    /// Позиция конца строки `line`: начало перевода строки, а для последней —
    /// конец документа. Для `\r\n` это байт `\r`, а не `\n`: курсор не должен
    /// вставать внутрь графемы.
    pub fn end_of_line(&self, line: u32, doc_len: u32) -> u32 {
        match self.breaks.get(line as usize) {
            Some(line_break) => line_break.start,
            None => doc_len,
        }
    }

    /// Все позиции начал переводов строк — для диагностики и тестов.
    pub fn breaks(&self) -> Vec<u32> {
        self.breaks
            .iter()
            .map(|line_break| line_break.start)
            .collect()
    }
}

/// Отдельные байты `\r` и `\n` с абсолютными позициями.
fn scan_atoms(text: &[u8], base: u32) -> Vec<(u32, bool)> {
    text.iter()
        .enumerate()
        .filter_map(|(at, byte)| match byte {
            b'\r' => Some((base + at as u32, true)),
            b'\n' => Some((base + at as u32, false)),
            _ => None,
        })
        .collect()
}

/// Склеить соседние `\r\n` в один перевод. Остальные байты остаются как есть.
fn group_atoms(atoms: &[(u32, bool)]) -> Vec<LineBreak> {
    let mut breaks = Vec::new();
    let mut at = 0;
    while at < atoms.len() {
        let (pos, cr) = atoms[at];
        if cr
            && let Some((next_pos, next_cr)) = atoms.get(at + 1)
            && !next_cr
            && *next_pos == pos + 1
        {
            breaks.push(LineBreak {
                start: pos,
                kind: BreakKind::CrLf,
            });
            at += 2;
            continue;
        }
        breaks.push(LineBreak {
            start: pos,
            kind: if cr { BreakKind::Cr } else { BreakKind::Lf },
        });
        at += 1;
    }
    breaks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Правки в эталоне: те же операции обычной строкой.
    fn reference(text: &str) -> LineMap {
        LineMap::build(text.as_bytes())
    }

    #[test]
    fn an_empty_document_has_one_line() {
        let map = LineMap::new();
        assert_eq!(map.line_count(), 1);
        assert_eq!(map.line_of(0), 0);
        assert_eq!(map.offset_of_line(0, 0), 0);
    }

    #[test]
    fn lines_are_found_by_position() {
        // Позиции считаются в байтах, поэтому в тестах латиница: у кириллицы
        // буква — два байта, и «четвёртый байт» там внутри слова.
        let map = reference("one\ntwo\nthree");
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_of(0), 0, "начало первой строки");
        assert_eq!(map.line_of(4), 1, "начало второй строки");
        assert_eq!(map.line_of(8), 2, "начало третьей строки");
        assert_eq!(map.line_of(13), 2, "конец документа в третьей строке");
    }

    #[test]
    fn a_position_after_the_break_belongs_to_the_next_line() {
        // Нажатие Enter переводит курсор на новую строку, и он должен быть
        // на ней: номер строки не отстаёт на единицу.
        let map = reference("one\ntwo");
        assert_eq!(map.line_of(3), 0, "сам перевод строки ещё конец первой");
        assert_eq!(map.line_of(4), 1, "первый байт следующей строки");
    }

    #[test]
    fn lines_are_counted_in_bytes_not_letters() {
        // Кириллица — два байта на букву, и карта обязана это знать: иначе
        // номер строки уедет на любом русском тексте.
        let text = "раз\nдва";
        let map = reference(text);
        let first = text.find('\n').unwrap() as u32;
        assert_eq!(map.line_of(first), 0, "перевод строки ещё в первой строке");
        assert_eq!(map.line_of(first + 1), 1, "начало второй строки");
    }

    #[test]
    fn line_numbers_go_to_positions_and_back() {
        let map = reference("one\ntwo\nthree\nfour");
        let len = 18u32;
        for line in 0..map.line_count() {
            let start = map.offset_of_line(line, len);
            assert_eq!(map.line_of(start), line, "строка {line} не сошлась обратно");
        }
        // За пределами документа — конец, а не паника.
        assert_eq!(map.offset_of_line(99, len), len);
    }

    #[test]
    fn inserting_a_break_adds_a_line() {
        let mut map = reference("one\nthree");
        map.insert(3, b"\n");
        assert_eq!(map.line_count(), 3, "стало три строки");
        assert_eq!(map.line_of(3), 0, "новый перевод — конец первой");
        assert_eq!(map.line_of(4), 1);
    }

    #[test]
    fn inserting_text_without_breaks_only_shifts() {
        let mut map = reference("one\nthree");
        map.insert(3, b"two");
        assert_eq!(map.line_count(), 2, "строк не добавилось");
        // «one» + «two» — шесть байт, перевод уехал с 3 на 6.
        assert_eq!(map.line_of(6), 0, "сам перевод ещё конец первой строки");
        assert_eq!(map.line_of(7), 1, "вторая строка сдвинулась");
    }

    #[test]
    fn deleting_a_break_removes_a_line() {
        let mut map = reference("one\ntwo\nthree");
        map.delete(3, 4);
        assert_eq!(map.line_count(), 2, "стало две строки");
        assert_eq!(map.line_of(3), 0, "всё слиплось в первую строку");
        // После удаления строки стали «onetwo» и «three», перевод на 7.
        assert_eq!(map.line_of(7), 1);
    }

    #[test]
    fn deleting_several_breaks_at_once() {
        let mut map = reference("a\nb\nc\nd\ne");
        map.delete(1, 7);
        assert_eq!(map.line_count(), 2, "из пяти строк осталось две");
        assert_eq!(map.line_of(1), 0);
    }

    #[test]
    fn insert_before_everything_shifts_all_breaks() {
        let mut map = reference("one\ntwo\nthree");
        map.insert(0, b"zero\n");
        assert_eq!(map.line_count(), 4);
        assert_eq!(map.line_of(0), 0);
        assert_eq!(map.line_of(5), 1);
        assert_eq!(map.line_of(9), 2);
    }

    #[test]
    fn a_rebuild_matches_the_incremental_path() {
        // Карта, собранная по дельтам, обязана совпадать с построенной с нуля:
        // иначе она тихо разъезжается на длинном документе.
        let base = "alpha\nbeta\ngamma\ndelta\n";
        let mut map = LineMap::build(base.as_bytes());
        let mut text = base.to_string();

        for step in 0..300usize {
            match step % 3 {
                0 => {
                    let middle = text.len() / 2;
                    map.insert(middle as u32, b"\n");
                    text.insert(middle, '\n');
                }
                1 => {
                    let third = (text.len() / 3).min(text.len().saturating_sub(1));
                    map.delete(third as u32, third as u32 + 1);
                    text.remove(third);
                }
                _ => {
                    let end = text.len();
                    map.insert(end as u32, b"line\n");
                    text.push_str("line\n");
                }
            }
            let fresh = LineMap::build(text.as_bytes());
            assert_eq!(
                map.breaks(),
                fresh.breaks(),
                "на шаге {step} карта разошлась с текстом"
            );
        }
    }

    #[test]
    fn end_of_line_points_at_the_break() {
        let map = reference("one\ntwo");
        assert_eq!(map.end_of_line(0, 7), 3, "конец первой строки — перевод");
        assert_eq!(
            map.end_of_line(1, 7),
            7,
            "конец последней — конец документа"
        );
    }

    #[test]
    fn a_document_without_a_trailing_break_still_has_a_last_line() {
        // Файл, который заканчивается не переводом строки, — самая частая
        // форма файла. Карта должна считать последнюю строку, а не терять её.
        let map = reference("one\ntwo");
        assert_eq!(map.breaks(), [3]);
        assert_eq!(map.line_count(), 2);
        assert_eq!(map.offset_of_line(1, 7), 4);
        assert_eq!(map.end_of_line(1, 7), 7);
    }

    #[test]
    fn deleting_to_the_end_removes_the_tail_breaks() {
        let mut map = reference("a\nb\nc\n");
        // Диапазон, а не длина: с 2-го байта до конца документа.
        map.delete(2, 6);
        assert_eq!(map.breaks(), [1], "остался только первый перевод");
        assert_eq!(map.line_count(), 2);
    }

    #[test]
    fn inserting_past_the_end_becomes_the_end() {
        // Позиция за пределами текста — это конец документа, а не поломка.
        // Карта не хранит переводы строк за текстом.
        let mut map = reference("a\nb");
        map.insert(99, b"\n");
        assert_eq!(map.breaks(), [1, 3]);
        assert_eq!(map.offset_of_line(2, 4), 4);
        assert_eq!(map.end_of_line(1, 4), 3);
    }

    #[test]
    fn a_line_number_past_the_document_is_the_end() {
        let map = reference("one\ntwo");
        assert_eq!(map.offset_of_line(99, 7), 7, "выше конца не прыгаем");
        assert_eq!(map.end_of_line(99, 7), 7);
        assert_eq!(map.line_of(99), 1, "позиция за текстом — последняя строка");
    }

    #[test]
    fn an_empty_document_has_no_breaks_at_all() {
        let map = reference("");
        assert!(map.breaks().is_empty());
        assert_eq!(map.line_count(), 1);
        assert_eq!(map.line_of(0), 0);
        assert_eq!(map.offset_of_line(0, 0), 0);
        assert_eq!(map.end_of_line(0, 0), 0);
    }

    #[test]
    fn crlf_is_one_break_and_its_end_is_the_cr() {
        let text = "a\r\nb\r\nc";
        let map = reference(text);
        assert_eq!(map.breaks(), [1, 4]);
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_of(1), 0, "сам \\r ещё в первой строке");
        assert_eq!(map.line_of(2), 0, "\\n внутри того же перевода");
        assert_eq!(map.line_of(3), 1, "после \\r\\n начинается следующая");
        assert_eq!(map.end_of_line(0, 7), 1, "конец строки — байт \\r");
        assert_eq!(map.end_of_line(1, 7), 4);
        assert_eq!(map.offset_of_line(1, 7), 3);
        assert_eq!(map.offset_of_line(2, 7), 6);
        assert_eq!(map.offset_of_line(3, 7), 7);
    }

    #[test]
    fn a_lone_cr_is_a_break() {
        let text = "a\rb\rc";
        let map = reference(text);
        assert_eq!(map.breaks(), [1, 3]);
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_of(1), 0);
        assert_eq!(map.line_of(2), 1);
        assert_eq!(map.end_of_line(0, 5), 1);
        assert_eq!(map.offset_of_line(1, 5), 2);
        assert_eq!(map.end_of_line(1, 5), 3);
        assert_eq!(map.offset_of_line(2, 5), 4);
    }

    #[test]
    fn inserting_an_lf_after_cr_merges_into_crlf() {
        let mut map = reference("a\rb");
        map.insert(2, b"\n");
        assert_eq!(map.breaks(), [1]);
        assert_eq!(map.line_count(), 2);
        assert_eq!(map.offset_of_line(1, 4), 3);
    }

    #[test]
    fn inserting_a_cr_before_lf_merges_into_crlf() {
        let mut map = reference("a\nb");
        map.insert(1, b"\r");
        assert_eq!(map.breaks(), [1]);
        assert_eq!(map.line_count(), 2);
        assert_eq!(map.offset_of_line(1, 4), 3);
    }

    #[test]
    fn deleting_between_cr_and_lf_merges_them() {
        let mut map = reference("a\rX\nb");
        map.delete(2, 3);
        assert_eq!(map.breaks(), [1]);
        assert_eq!(map.line_count(), 2);
        assert_eq!(map.offset_of_line(1, 4), 3);
    }

    #[test]
    fn deleting_half_of_crlf_leaves_the_other_half() {
        let mut without_cr = reference("a\r\nb");
        without_cr.delete(1, 2);
        assert_eq!(without_cr.breaks(), [1], "остался \\n");
        assert_eq!(without_cr.offset_of_line(1, 3), 2);

        let mut without_lf = reference("a\r\nb");
        without_lf.delete(2, 3);
        assert_eq!(without_lf.breaks(), [1], "остался \\r");
        assert_eq!(without_lf.end_of_line(0, 3), 1);
        assert_eq!(without_lf.offset_of_line(1, 3), 2);
    }

    #[test]
    fn inserting_inside_crlf_splits_it() {
        let mut map = reference("a\r\nb");
        map.insert(2, b"x");
        assert_eq!(map.breaks(), [1, 3]);
        assert_eq!(map.line_count(), 3);
        assert_eq!(map.line_of(2), 1);
        assert_eq!(map.end_of_line(1, 5), 3);
    }

    #[test]
    fn mixed_break_edits_match_a_rebuild() {
        let mut map = reference("a\r\nb\rc\nd");
        let mut text = "a\r\nb\rc\nd".to_string();
        let mut seed = 7u32;
        for step in 0..200 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let pos = (seed as usize) % (text.len() + 1);
            if seed.is_multiple_of(3) && pos < text.len() {
                let to = (pos + 1).min(text.len());
                map.delete(pos as u32, to as u32);
                text.replace_range(pos..to, "");
            } else {
                let byte = match seed % 3 {
                    0 => '\r',
                    1 => '\n',
                    _ => 'x',
                };
                map.insert(pos as u32, &[byte as u8]);
                text.insert(pos, byte);
            }
            let fresh = LineMap::build(text.as_bytes());
            assert_eq!(
                map.breaks(),
                fresh.breaks(),
                "на шаге {step} карта разошлась с текстом {text:?}"
            );
            assert_eq!(map.line_count(), fresh.line_count());
        }
    }
}

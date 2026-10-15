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

/// Карта переводов строк.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineMap {
    /// Позиции переводов строк по возрастанию.
    ///
    /// Список, а не дерево: сдвиг при вставке в начало стоит копирования
    /// хвоста, но это единицы мегабайт на правку и ни байта памяти сверх
    /// четырёх на перевод строки.
    breaks: Vec<u32>,
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
        self.breaks.clear();
        self.breaks.extend(
            text.iter()
                .enumerate()
                .filter_map(|(i, b)| (*b == b'\n').then_some(i as u32)),
        );
    }

    /// Вставка текста: `len` байт на позицию `pos`.
    ///
    /// Переводы строк в куске вставляются на свои места, а всё, что стояло
    /// дальше, сдвигается на длину куска.
    pub fn insert(&mut self, pos: u32, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        // Сдвигаются все, кто стоит не раньше места вставки: и те, что под
        // вставкой оказались, и те, что за ней. Забыть последних — значит
        // через пару правок карта уедет.
        let first = self.breaks.partition_point(|b| *b < pos);
        let len = data.len() as u32;
        self.breaks[first..].iter_mut().for_each(|b| *b += len);

        let mut added = Vec::with_capacity(data.len() / 24);
        let offset = pos as usize;
        for (i, b) in data.iter().enumerate() {
            if *b == b'\n' {
                added.push((offset + i) as u32);
            }
        }
        self.breaks.splice(first..first, added);
    }

    /// Удаление диапазона `from..to`: его длина до удаления.
    ///
    /// Переводы строк внутри пропадают вместе с текстом, а всё, что стояло
    /// дальше, сдвигается влево на длину удалённого.
    pub fn delete(&mut self, from: u32, to: u32) {
        if to <= from {
            return;
        }
        let first = self.breaks.partition_point(|b| *b < from);
        let last = self.breaks.partition_point(|b| *b < to);
        let removed = to - from;
        self.breaks.drain(first..last);
        // Сдвигать надо от `first`, а не от `last`: после удаления хвост
        // сдвинулся к началу, и старый индекс указывал бы за его конец.
        self.breaks[first..].iter_mut().for_each(|b| *b -= removed);
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
        self.breaks.partition_point(|b| *b < pos) as u32
    }

    /// Позиция, с которой начинается строка `line`, начиная с нуля.
    ///
    /// Перевод с индексом `k` стоит в конце строки `k`, поэтому строка `k + 1`
    /// начинается с него. За пределами документа — конец документа: прыгать
    /// выше конца бессмысленно.
    pub fn offset_of_line(&self, line: u32, doc_len: u32) -> u32 {
        if line == 0 {
            return 0;
        }
        match self.breaks.get(line as usize - 1) {
            Some(b) => b + 1,
            None => doc_len,
        }
    }

    /// Позиция конца строки `line`: сам перевод строки, а для последней —
    /// конец документа.
    pub fn end_of_line(&self, line: u32, doc_len: u32) -> u32 {
        match self.breaks.get(line as usize) {
            Some(b) => *b,
            None => doc_len,
        }
    }

    /// Все позиции переводов строк — для диагностики и тестов.
    pub fn breaks(&self) -> &[u32] {
        &self.breaks
    }
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
            let at = map.offset_of_line(line, len);
            assert_eq!(map.line_of(at), line, "строка {line} не сошлась обратно");
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
                    let at = text.len() / 2;
                    map.insert(at as u32, b"\n");
                    text.insert(at, '\n');
                }
                1 => {
                    let at = (text.len() / 3).min(text.len().saturating_sub(1));
                    map.delete(at as u32, at as u32 + 1);
                    text.remove(at);
                }
                _ => {
                    let at = text.len();
                    map.insert(at as u32, b"line\n");
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
}

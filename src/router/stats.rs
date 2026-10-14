//! Статистика работы: что человек делал с файлом.
//!
//! Живёт отдельно от документа и от сессии: это счётчик, а не часть правки.
//! Ни одно из этих чисел не влияет на текст и не решает за пользователя, что
//! хорошо, а что плохо.
//!
//! Время нажатия считается так, как его видит терминал: от `KeyPressed` до
//! `KeyReleased` с тем же ключом. Это и есть «сколько реально писал» — сумма
//! таких промежутков. Секунды на размышление между нажатиями сюда не попадают:
//! иначе час правки двух абзацев выглядел бы как восемь часов работы.
//!
//! Статистика живёт только в памяти и умирает вместе с сеансом. Сохранять её
//! незачем: это не счётчик достижений.

use std::time::Duration;

/// Что собрано о работе над документом за сеанс.
///
/// Поля копируемые: снимок статистики — обычное число, его можно показать на
/// экране после сохранения или просто выбросить.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Сколько символов введено.
    pub input_chars: u64,
    /// Сколько символов удалено.
    pub deleted_chars: u64,
    /// Сколько нажатий клавиш сделано.
    pub presses: u64,
    /// Сколько времени заняли сами нажатия.
    pub pressed: Duration,
    /// Сколько правок сделано.
    pub edits: u64,
}

impl Stats {
    /// Занято истории, процент.
    ///
    /// Больше сотни не бывает: сколько бы правок ни сделано, сотня уже
    /// значит, что историю пора сбрасывать.
    pub fn history_percent(&self, used: u32, limit: u32) -> u32 {
        if limit == 0 {
            return 0;
        }
        (used * 100 / limit).min(100)
    }

    /// Сколько записей истории ещё можно положить, без дна.
    ///
    /// Остаток может оказаться отрицательным, и это правда: история растёт по
    /// мере правок, а не вмещается в отведённое заранее место.
    pub fn history_remaining(&self, used: u32, limit: u32) -> i64 {
        i64::from(limit) - i64::from(used)
    }
}

/// Сколько символов в куске байт.
///
/// Символов, а не байт: русская буква — два байта, и по байтам статистика
/// врала бы вдвое на всём, кроме латиницы. Разбор делается на вставленном
/// куске, он мелкий; если это не текст, считаются байты.
pub fn chars(data: &[u8]) -> u64 {
    match std::str::from_utf8(data) {
        Ok(text) => text.chars().count() as u64,
        Err(_) => data.len() as u64,
    }
}

/// Записать ввод символов в накопленную статистику.
pub fn note_input(stats: &mut Stats, data: &[u8]) {
    stats.input_chars += chars(data);
    stats.edits += 1;
}

/// Записать удаление символов.
pub fn note_deleted(stats: &mut Stats, len: u32) {
    stats.deleted_chars += u64::from(len);
    stats.edits += 1;
}

/// Записать замену: одна правка, состоящая из удаления и вставки.
///
/// Считается одним действием, а не двумя: пользователь нажал одну команду, и
/// в истории она тоже одна запись.
pub fn note_replace(stats: &mut Stats, data: &[u8], removed: u32) {
    stats.input_chars += chars(data);
    stats.deleted_chars += u64::from(removed);
    stats.edits += 1;
}

/// Записать одно нажатие вместе со временем, которое оно заняло.
///
/// Нажатие без отпускания — обычное дело: палец на клавише или потерянное
/// событие. Такое нажатие засчитывается, но времени не добавляет.
pub fn note_press(stats: &mut Stats, held: Duration) {
    stats.presses += 1;
    stats.pressed += held;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_deleting_are_counted_separately() {
        let mut s = Stats::default();
        note_input(&mut s, "абв".as_bytes());
        note_deleted(&mut s, 2);
        note_input(&mut s, "г".as_bytes());
        // Буквы, а не байты: «абв» это три, «г» это одна.
        assert_eq!(s.input_chars, 4);
        assert_eq!(s.deleted_chars, 2);
        assert_eq!(s.edits, 3);
    }

    #[test]
    fn only_the_held_time_counts_not_the_gap() {
        // Пауза между нажатиями — это размышление, а не печать.
        let mut s = Stats::default();
        note_press(&mut s, Duration::from_millis(80));
        // Между нажатиями прошло двадцать секунд: они сюда не попадают.
        note_press(&mut s, Duration::from_millis(120));
        assert_eq!(s.presses, 2);
        assert_eq!(s.pressed, Duration::from_millis(200));
    }

    #[test]
    fn a_press_that_never_released_adds_no_time() {
        let mut s = Stats::default();
        note_press(&mut s, Duration::ZERO);
        assert_eq!(s.presses, 1, "нажатие засчитано");
        assert_eq!(s.pressed, Duration::ZERO, "времени не добавило");
    }

    #[test]
    fn a_replace_is_one_edit() {
        let mut s = Stats::default();
        note_replace(&mut s, "другая".as_bytes(), 12);
        assert_eq!(s.edits, 1, "замена — одна правка");
        assert_eq!(s.input_chars, 6);
        assert_eq!(s.deleted_chars, 12);
    }

    #[test]
    fn history_percent_is_never_above_a_hundred() {
        let s = Stats::default();
        assert_eq!(
            s.history_percent(900, 100),
            100,
            "процент упирается в сотню"
        );
        assert_eq!(
            s.history_remaining(900, 100),
            -800,
            "остаток бывает отрицательным"
        );
        assert_eq!(s.history_percent(40, 100), 40);
        assert_eq!(s.history_percent(10, 0), 0, "без размера процент нулевой");
    }

    #[test]
    fn binary_junk_is_counted_as_bytes() {
        // Не текст — значит считаем байты, а не буквы: иначе счётчик врёт.
        let junk = [0xffu8, 0xfe, 0x00];
        assert_eq!(chars(&junk), 3);
    }
}

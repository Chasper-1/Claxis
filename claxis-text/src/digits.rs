//! Разбивка чисел на разряды.

/// Разделитель разрядов в числах.
///
/// Пробел: он читается одинаково в любом языке и не путается с точкой в
/// десятичной дроби, в отличие от точки или запятой.
const THOUSANDS: char = ' ';

/// Разбить число на разряды: `4294967295` становится `4 294 967 295`.
///
/// Без разбивки длинное число не читается — не видно, где миллиард. Правило
/// одно на все сообщения редактора.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    if digits.len() <= 3 {
        return digits;
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        // Разделитель ставим там, где справа остаётся кратное трём число цифр.
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(THOUSANDS);
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::group_digits;

    #[test]
    fn digits_are_grouped_by_three() {
        assert_eq!(group_digits(4_294_967_295), "4 294 967 295");
        assert_eq!(group_digits(1_000_000_000), "1 000 000 000");
        assert_eq!(group_digits(1_234_567_890), "1 234 567 890");
    }

    #[test]
    fn short_numbers_stay_whole() {
        // Разделять нужно только длинные: `9 9 9` читается хуже, чем `999`.
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(100), "100");
        assert_eq!(group_digits(0), "0");
    }

    #[test]
    fn grouping_never_loses_digits() {
        // Ошибка в разбивке портит число молча, поэтому проверяем по кругу.
        for n in [1_000u64, 999_999, 1_000_001, 4_294_967_295, 12_345] {
            let joined: String = group_digits(n).chars().filter(|c| *c != ' ').collect();
            assert_eq!(joined, n.to_string(), "цифры разъехались у {n}");
        }
    }

    #[test]
    fn every_digit_length_is_grouped_from_the_right() {
        // Граница между тройками и нет: 4, 5, 6 и 7 цифр по-разному делятся.
        assert_eq!(group_digits(1_000), "1 000");
        assert_eq!(group_digits(10_000), "10 000");
        assert_eq!(group_digits(100_000), "100 000");
        assert_eq!(group_digits(1_000_000), "1 000 000");
        assert_eq!(group_digits(10_000_000), "10 000 000");
        assert_eq!(group_digits(u64::MAX), "18 446 744 073 709 551 615");
    }
}

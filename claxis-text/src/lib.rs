//! Работа с текстом: общие примитивы, которыми пользуются все крейты.
//!
//! Сюда кладутся функции, которые нужны не одному крейту, а сразу всем, и
//! которые иначе пришлось бы копировать из крейта в крейт. Правило одно: что
//! нужно поправить — правится здесь, а не в четырёх местах сразу.

/// Подставить значения вместо имён в фигурных скобках.
///
/// Места под значения — часть перевода: в каталоге написано
/// `пул исчерпан: узлов {capacity}`, и на экране будет `пул исчерпан: узлов
/// 408`. Переводчик ставит имена туда, куда надо по правилам своего языка, а
/// эта функция подставляет вместо них значения.
pub fn substitute(text: &str, values: &[(&str, String)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

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
    use super::{group_digits, substitute};

    #[test]
    fn substitute_replaces_names() {
        let text = "range {anchor}..{end}";
        let out = substitute(
            text,
            &[("anchor", "5".to_string()), ("end", "9".to_string())],
        );
        assert_eq!(out, "range 5..9");
    }

    #[test]
    fn substitute_leaves_unknown_names_alone() {
        // Неизвестное имя проверяет разбор перевода, а не подстановка: молча
        // выбрасывать его нельзя, иначе потеряется причина ошибки.
        let out = substitute("a {known} b {unknown}", &[("known", "1".to_string())]);
        assert_eq!(out, "a 1 b {unknown}");
    }

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
}

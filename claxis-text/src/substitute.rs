//! Подстановка значений в шаблон сообщения.

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

#[cfg(test)]
mod tests {
    use super::substitute;

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
    fn a_name_repeated_twice_is_filled_in_both_places() {
        // Перевод может упомянуть одно значение дважды: заполняются оба места.
        let out = substitute("{a} и {a}", &[("a", "раз".to_string())]);
        assert_eq!(out, "раз и раз");
    }

    #[test]
    fn a_value_with_braces_is_inserted_as_is() {
        // Значение приходит из данных: скобки в нём не должны превращаться в
        // новые места для подстановки.
        let out = substitute("файл {path}", &[("path", "{weird}".to_string())]);
        assert_eq!(out, "файл {weird}");
    }

    #[test]
    fn an_empty_value_leaves_the_text_shorter() {
        let out = substitute("узлов: {n}", &[("n", String::new())]);
        assert_eq!(out, "узлов: ");
    }

    #[test]
    fn nothing_to_substitute_gives_the_same_text() {
        let out = substitute("обычный текст без мест", &[]);
        assert_eq!(out, "обычный текст без мест");
    }

    #[test]
    fn a_value_containing_another_name_does_not_expand_further() {
        // Подстановка идёт по порядку: значение не должно запускать новую
        // подстановку внутри себя.
        let out = substitute(
            "{a} {b}",
            &[("a", "{b}".to_string()), ("b", "2".to_string())],
        );
        assert_eq!(out, "2 2");
    }
}

//! Код цвета из темы или конфига.
//!
//! Поддерживаются только привычные записи: hex и функция `rgb`/`rgba`.
//! Имён и ссылок на палитру терминала здесь нет: у цвета одно значение, и
//! оно одинаково читается в файле и в подсветке конфига.
//!
//! Альфа необязательна: без неё цвет непрозрачный. Внутри она хранится
//! байтом `0..=255`, а пишется либо долей `0..=1`, либо процентами.

use crate::Error;

/// Цвет: красный, зелёный, синий и альфа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// Красный канал.
    pub r: u8,
    /// Зелёный канал.
    pub g: u8,
    /// Синий канал.
    pub b: u8,
    /// Непрозрачность: `255` — цвет виден полностью.
    pub a: u8,
}

impl Color {
    /// Непрозрачная альфа.
    pub const OPAQUE: u8 = u8::MAX;

    /// Разобрать код цвета.
    ///
    /// Hex: `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, регистр любой.
    /// Функция: `rgb(r, g, b)` и `rgba(r, g, b, a)`, каналы `0..=255`,
    /// альфа — доля `0..=1` или проценты.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let text = text.trim();
        if let Some(digits) = text.strip_prefix('#') {
            return Self::hex(digits, text);
        }
        let open = text.find('(').ok_or_else(|| Error::NotAColor {
            value: text.to_string(),
        })?;
        let body = text.strip_suffix(')').ok_or_else(|| Error::NotAColor {
            value: text.to_string(),
        })?;
        let body = &body[open + 1..];
        match text[..open].to_ascii_lowercase().as_str() {
            "rgb" => Self::function(body, 3, text),
            "rgba" => Self::function(body, 4, text),
            _ => Err(Error::NotAColor {
                value: text.to_string(),
            }),
        }
    }

    /// Цвет из hex-цифр без решётки.
    fn hex(digits: &str, text: &str) -> Result<Self, Error> {
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::NotAColor {
                value: text.to_string(),
            });
        }
        let chars: Vec<char> = digits.chars().collect();
        let (r, g, b, a) = match chars.len() {
            3 => (
                nibble(chars[0], text)?,
                nibble(chars[1], text)?,
                nibble(chars[2], text)?,
                Self::OPAQUE,
            ),
            4 => (
                nibble(chars[0], text)?,
                nibble(chars[1], text)?,
                nibble(chars[2], text)?,
                nibble(chars[3], text)?,
            ),
            6 => (
                pair(&digits[0..2], text)?,
                pair(&digits[2..4], text)?,
                pair(&digits[4..6], text)?,
                Self::OPAQUE,
            ),
            8 => (
                pair(&digits[0..2], text)?,
                pair(&digits[2..4], text)?,
                pair(&digits[4..6], text)?,
                pair(&digits[6..8], text)?,
            ),
            _ => {
                return Err(Error::NotAColor {
                    value: text.to_string(),
                });
            }
        };
        Ok(Self { r, g, b, a })
    }

    /// Цвет из тела `rgb(...)` или `rgba(...)`.
    fn function(body: &str, parts: usize, text: &str) -> Result<Self, Error> {
        let chunks: Vec<&str> = body.split(',').collect();
        if chunks.len() != parts {
            return Err(Error::NotAColor {
                value: text.to_string(),
            });
        }
        let r = channel(chunks[0], text)?;
        let g = channel(chunks[1], text)?;
        let b = channel(chunks[2], text)?;
        let a = if parts == 4 {
            alpha(chunks[3], text)?
        } else {
            Self::OPAQUE
        };
        Ok(Self { r, g, b, a })
    }
}

/// Канал `0..=255`.
fn channel(part: &str, text: &str) -> Result<u8, Error> {
    let value = part.trim().parse::<u32>().map_err(|_| Error::NotAColor {
        value: text.to_string(),
    })?;
    if value > u32::from(u8::MAX) {
        return Err(Error::ColorChannel {
            value: text.to_string(),
        });
    }
    Ok(value as u8)
}

/// Альфа: доля `0..=1` или проценты.
fn alpha(part: &str, text: &str) -> Result<u8, Error> {
    let part = part.trim();
    if let Some(percent) = part.strip_suffix('%') {
        let value = percent
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::NotAColor {
                value: text.to_string(),
            })?;
        if !(0.0..=100.0).contains(&value) {
            return Err(Error::ColorChannel {
                value: text.to_string(),
            });
        }
        return Ok((value * 255.0 / 100.0).round() as u8);
    }
    let value = part.parse::<f64>().map_err(|_| Error::NotAColor {
        value: text.to_string(),
    })?;
    if !(0.0..=1.0).contains(&value) {
        return Err(Error::ColorChannel {
            value: text.to_string(),
        });
    }
    Ok((value * 255.0).round() as u8)
}

/// Одна hex-цифра: `f` → `255`, `0` → `0`.
fn nibble(digit: char, text: &str) -> Result<u8, Error> {
    let value = digit.to_digit(16).ok_or_else(|| Error::NotAColor {
        value: text.to_string(),
    })?;
    Ok((value * 17) as u8)
}

/// Пара hex-цифр.
fn pair(digits: &str, text: &str) -> Result<u8, Error> {
    u8::from_str_radix(digits, 16).map_err(|_| Error::NotAColor {
        value: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::Color;
    use crate::Error;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color {
            r,
            g,
            b,
            a: Color::OPAQUE,
        }
    }

    #[test]
    fn long_hex_is_read_in_three_channels() {
        assert_eq!(Color::parse("#2d353b").unwrap(), rgb(0x2d, 0x35, 0x3b));
        assert_eq!(Color::parse("#FFFFFF").unwrap(), rgb(255, 255, 255));
        assert_eq!(Color::parse("#000000").unwrap(), rgb(0, 0, 0));
    }

    #[test]
    fn hex_letter_case_does_not_matter() {
        assert_eq!(Color::parse("#aBcDeF").unwrap(), rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn short_hex_expands_each_digit() {
        assert_eq!(Color::parse("#abc").unwrap(), rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(Color::parse("#f0a").unwrap(), rgb(0xff, 0x00, 0xaa));
    }

    #[test]
    fn short_hex_with_alpha_reads_four_digits() {
        assert_eq!(
            Color::parse("#abcd").unwrap(),
            Color {
                r: 0xaa,
                g: 0xbb,
                b: 0xcc,
                a: 0xdd
            }
        );
    }

    #[test]
    fn long_hex_with_alpha_reads_eight_digits() {
        assert_eq!(
            Color::parse("#2d353b80").unwrap(),
            Color {
                r: 0x2d,
                g: 0x35,
                b: 0x3b,
                a: 0x80
            }
        );
    }

    #[test]
    fn rgb_is_opaque() {
        assert_eq!(Color::parse("rgb(1, 2, 3)").unwrap(), rgb(1, 2, 3));
        assert_eq!(Color::parse("RGB(255, 0, 10)").unwrap(), rgb(255, 0, 10));
    }

    #[test]
    fn spaces_around_function_arguments_are_allowed() {
        assert_eq!(
            Color::parse("rgba( 1 , 2 , 3 , 0.5 )").unwrap(),
            Color {
                r: 1,
                g: 2,
                b: 3,
                a: 128
            }
        );
    }

    #[test]
    fn alpha_accepts_a_fraction() {
        assert_eq!(Color::parse("rgba(0, 0, 0, 0)").unwrap().a, 0);
        assert_eq!(Color::parse("rgba(0, 0, 0, 1)").unwrap().a, 255);
        assert_eq!(Color::parse("rgba(0, 0, 0, 0.5)").unwrap().a, 128);
    }

    #[test]
    fn alpha_accepts_percent() {
        assert_eq!(Color::parse("rgba(0, 0, 0, 0%)").unwrap().a, 0);
        assert_eq!(Color::parse("rgba(0, 0, 0, 100%)").unwrap().a, 255);
        assert_eq!(Color::parse("rgba(0, 0, 0, 50%)").unwrap().a, 128);
    }

    #[test]
    fn surrounding_spaces_are_ignored() {
        assert_eq!(Color::parse("  #010203  ").unwrap(), rgb(1, 2, 3));
        assert_eq!(Color::parse("  rgb(1,2,3)  ").unwrap(), rgb(1, 2, 3));
    }

    #[test]
    fn an_unknown_format_is_an_error() {
        for text in [
            "",
            "red",
            "#",
            "#12345",
            "hsl(1,2,3)",
            "rgb(1,2)",
            "rgba(1,2,3)",
        ] {
            assert!(
                matches!(Color::parse(text), Err(Error::NotAColor { .. })),
                "{text:?} должно быть отбито"
            );
        }
    }

    #[test]
    fn a_non_hex_digit_is_an_error() {
        for text in ["#xyz", "#12g", "#1234z5", "rgb(a,2,3)"] {
            assert!(
                matches!(Color::parse(text), Err(Error::NotAColor { .. })),
                "{text:?} должно быть отбито"
            );
        }
    }

    #[test]
    fn a_channel_above_255_is_out_of_range() {
        for text in ["rgb(256,0,0)", "rgb(0,300,0)", "rgba(0,0,999,1)"] {
            assert!(
                matches!(Color::parse(text), Err(Error::ColorChannel { .. })),
                "{text:?} должно быть отбито"
            );
        }
    }

    #[test]
    fn alpha_outside_its_range_is_an_error() {
        for text in [
            "rgba(0,0,0,1.5)",
            "rgba(0,0,0,-0.1)",
            "rgba(0,0,0,101%)",
            "rgba(0,0,0,NaN)",
        ] {
            assert!(
                matches!(Color::parse(text), Err(Error::ColorChannel { .. })),
                "{text:?} должно быть отбито"
            );
        }
    }

    #[test]
    fn a_negative_channel_is_not_a_number() {
        assert!(matches!(
            Color::parse("rgb(-1,0,0)"),
            Err(Error::NotAColor { .. })
        ));
    }

    #[test]
    fn the_error_text_goes_through_the_catalog() {
        let err = Color::parse("#xyz").unwrap_err();
        assert!(err.text().contains("#xyz"), "{}", err.text());
        let err = Color::parse("rgb(300,0,0)").unwrap_err();
        assert!(err.text().contains("300"), "{}", err.text());
    }
}

//! Контрольная сумма содержимого.
//!
//! Нужна, чтобы отличить две вещи, которые снапшот не может рассказать сам:
//! записали мы не то, или ему испортили снаружи. После записи содержимое
//! совпадает всегда — если только его не переписали между проверкой и
//! использованием. Сумма позволяет узнать это раньше, чем человек увидит
//! битый текст.
//!
//! Считается FNV-1a на 64 бита: детерминированная, без зависимости от
//! платформы и без таблицы, и для этого хватает. Не защита от подделки — защита от
//! порчи.
//!
//! Файл целиком в память не читается: [`of_reader`] считает по кускам, чтобы
//! большой файл не пришлось держать целиком.

use std::io::Read;

/// Смещение FNV-1a.
const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// Множитель FNV-1a: простое число 2^40 + 2^8 + 0xb.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Сумма куска. Продолжение счёта, а не начало нового.
fn continue_with(mut sum: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        sum ^= u64::from(*byte);
        sum = sum.wrapping_mul(PRIME);
    }
    sum
}

/// Сумма содержимого.
pub fn of(bytes: &[u8]) -> u64 {
    continue_with(OFFSET, bytes)
}

/// Сумма двух кусков, как один: так считается снапшот из текста и истории.
///
/// Порядок важен и фиксирован: перестановка кусков даёт другую сумму, а
/// значит подмену видно.
pub fn of_pair(first: &[u8], second: &[u8]) -> u64 {
    // Длина первого куска входит в сумму: иначе `ab` + `c` и `a` + `bc`
    // совпали бы, а это разные снапшоты.
    let with_len = continue_with(OFFSET, &(first.len() as u64).to_le_bytes());
    continue_with(with_len, first).wrapping_add(of(second).rotate_left(1))
}

/// Сумма содержимого потока, по кускам.
pub fn of_reader(mut reader: impl Read) -> std::io::Result<u64> {
    let mut sum = OFFSET;
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(sum);
        }
        sum = continue_with(sum, &chunk[..read]);
    }
}

/// Сумма файла. `None`, если файла нет.
pub fn of_file(path: &std::path::Path) -> std::io::Result<Option<u64>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    of_reader(file).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_bytes_always_give_the_same_sum() {
        // В этом весь смысл: сумма детерминирована, иначе сравнивать нечего.
        assert_eq!(of("привет".as_bytes()), of("привет".as_bytes()));
        assert_eq!(of(b""), of(b""));
        assert_eq!(
            of_pair("текст".as_bytes(), "история".as_bytes()),
            of_pair("текст".as_bytes(), "история".as_bytes())
        );
    }

    #[test]
    fn different_bytes_give_different_sums() {
        assert_ne!(of("один".as_bytes()), of("два".as_bytes()));
        assert_ne!(of(b""), of(b"a"));
        assert_ne!(of(b"ab"), of(b"ba"));
    }

    #[test]
    fn a_snapshot_sum_covers_both_parts_without_swapping_them() {
        // Подмена истории при том же тексте — тоже порча, и сумма обязана её
        // видеть.
        assert_ne!(
            of_pair("текст".as_bytes(), "история".as_bytes()),
            of_pair("история".as_bytes(), "текст".as_bytes())
        );
        assert_ne!(
            of_pair("текст".as_bytes(), "история".as_bytes()),
            of_pair("текст".as_bytes(), "истор_я".as_bytes())
        );
        // Перестановка кусков не должна давать ту же сумму, что и склейка.
        assert_ne!(of_pair(b"ab", b"c"), of_pair(b"a", b"bc"));
    }

    #[test]
    fn a_sum_by_chunks_equals_a_sum_of_the_whole() {
        let text = "строка подлиннее, чтобы чтение шло несколькими кусками".repeat(200);
        let mut chunked = OFFSET;
        // Кусок заведомо меньше буфера внутри `of_reader`, чтобы чтение шло
        // не одним куском, а сумма собралась по частям.
        for part in text.as_bytes().chunks(7) {
            chunked = continue_with(chunked, part);
        }
        assert_eq!(of_reader(text.as_bytes()).unwrap(), chunked);
        assert_eq!(of_reader(text.as_bytes()).unwrap(), of(text.as_bytes()));
    }

    #[test]
    fn a_missing_file_has_no_sum_and_a_broken_one_is_an_error() {
        let dir = std::env::temp_dir().join("claxis-checksum-missing");
        let _ = std::fs::remove_file(&dir);
        assert_eq!(of_file(&dir).unwrap(), None);

        let path = std::env::temp_dir().join("claxis-checksum-dir");
        std::fs::create_dir_all(&path).unwrap();
        assert!(of_file(&path).is_err(), "каталог — не файл");
        std::fs::remove_dir_all(&path).unwrap();
    }
}

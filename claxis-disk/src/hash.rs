//! Хеш содержимого для проверки «файл менялся?».
//!
//! Это XXH3 на 128 бит: сумма не для подписи и не для защиты от зла, а для
//! того, чтобы две проверки — размер и хеш — вместе могли ответить, что файл
//! тот же. Такое совпадение у правки снаружи вероятность имеет еле заметную,
//! и платят за это скоростью: гигабайты считаются за доли секунды.
//!
//! Хеш считается и от того, что уже в памяти (буфер), и от файла на диске.
//! Диск читается по кускам: держать файл целиком ради проверки незачем.

use std::io::Read;
use std::path::Path;

use xxhash_rust::xxh3::{Xxh3, xxh3_128};

/// Хеш содержимого.
pub fn of(bytes: &[u8]) -> u128 {
    xxh3_128(bytes)
}

/// Хеш содержимого потока, по кускам.
pub fn of_reader(mut reader: impl Read) -> std::io::Result<u128> {
    let mut hasher = Xxh3::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hasher.digest128())
}

/// Хеш файла. `None`, если файла нет.
///
/// Отсутствие файла — не ошибка: файл могли ещё не создать.
pub fn of_file(path: &Path) -> crate::Result<Option<u128>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(crate::error::io(path, e)),
    };
    of_reader(file)
        .map(Some)
        .map_err(|e| crate::error::io(path, e))
}

/// Хеш как он лежит в базе: шестнадцать байт, старшие вперёд.
///
/// `u128` в базе напрямую sqlite не умеет, а байты — умеет всегда.
pub fn to_blob(hash: u128) -> Vec<u8> {
    hash.to_le_bytes().to_vec()
}

/// Байты из базы обратно в хеш. Также и проверяется длина: короткий блоб —
/// испорченная запись, а не ноль.
pub fn from_blob(bytes: &[u8]) -> Option<u128> {
    let bytes: [u8; 16] = bytes.try_into().ok()?;
    Some(u128::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_bytes_always_give_the_same_hash() {
        assert_eq!(of("привет".as_bytes()), of("привет".as_bytes()));
        assert_eq!(of(b""), of(b""));
    }

    #[test]
    fn different_bytes_give_different_hashes() {
        // Две правки в одном байте обязаны различаться: на этом держится вся
        // проверка, когда размер совпал.
        assert_ne!(of("один".as_bytes()), of("два".as_bytes()));
        assert_ne!(of(b""), of(b"a"));
    }

    #[test]
    fn a_hash_by_chunks_equals_a_hash_of_the_whole() {
        let text = "строка подлиннее, чтобы чтение шло несколькими кусками".repeat(300);
        assert_eq!(of_reader(text.as_bytes()).unwrap(), of(text.as_bytes()));
    }

    #[test]
    fn a_missing_file_has_no_hash() {
        let dir = std::env::temp_dir().join("claxis-hash-missing");
        let _ = std::fs::remove_file(&dir);
        assert_eq!(of_file(&dir).unwrap(), None);
    }

    #[test]
    fn a_file_hash_matches_its_bytes() {
        let dir = std::env::temp_dir();
        let path = dir.join("claxis-hash-bytes");
        std::fs::write(&path, b"same text").unwrap();
        assert_eq!(of_file(&path).unwrap(), Some(of(b"same text")));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_hash_survives_the_blob_round_trip() {
        let hash = of("содержимое на посадку".as_bytes());
        assert_eq!(from_blob(&to_blob(hash)), Some(hash));
    }

    #[test]
    fn a_shorted_blob_is_not_a_hash() {
        // Обрезка по дороге в базу — это испорченная запись, и молчаливым нулём
        // она притворяться не должна.
        assert_eq!(from_blob(&[0, 1, 2, 3]), None);
    }
}

//! Что не так с диском.

use std::fmt;

use crate::messages::Messages;

/// Ошибка работы с диском.
#[derive(Debug)]
pub enum Error {
    /// Файл не записан: `{path}`, `{reason}`.
    Write { path: String, reason: String },
    /// Файл не прочитан: `{path}`, `{reason}`.
    Read { path: String, reason: String },
    /// Каталог не создан: `{path}`, `{reason}`.
    PrepareDir { path: String, reason: String },
    /// Файл не переименован после записи: `{path}`, `{reason}`.
    Rename { path: String, reason: String },
    /// Подписка на каталог не вышла: `{path}`, `{reason}`.
    Watch { path: String, reason: String },
    /// Система наблюдения не ответила: `{reason}`.
    WatchFailed { reason: String },
    /// Файл базы не открылся: `{path}`, `{reason}`.
    Open { path: String, reason: String },
    /// Запрос к базе не выполнился: `{reason}`.
    Query { reason: String },
    /// Снапшота для файла нет: `{file}`.
    NoSnapshot {
        /// Какой файл.
        file: String,
    },
    /// Снапшот испорчен извне: `{file}`, `{seq}`, `{expected}`, `{got}`.
    ///
    /// Отдельная ошибка не ради вежливости: если запись виновата, снимок надо
    /// переписать, и всё. А если содержимое переписали после нас, перезапись
    /// бесполезна — она затрёт то, что человек сделал руками, и вот тут уже
    /// надо сказать человеку.
    Corrupted {
        /// Какой файл.
        file: String,
        /// Какой снапшот по счёту.
        seq: u64,
        /// Какая сумма должна была быть.
        expected: u64,
        /// Какая сумма оказалась.
        got: u64,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    pub fn message(&self, messages: &dyn Messages) -> String {
        match self {
            Error::Write { path, reason } => messages.write_failed(path, reason),
            Error::Read { path, reason } => messages.read_failed(path, reason),
            Error::PrepareDir { path, reason } => messages.prepare_dir_failed(path, reason),
            Error::Rename { path, reason } => messages.rename_failed(path, reason),
            Error::Watch { path, reason } => messages.watch_failed(path, reason),
            Error::WatchFailed { reason } => messages.watch_system_failed(reason),
            Error::Open { path, reason } => messages.open_failed(path, reason),
            Error::Query { reason } => messages.query_failed(reason),
            Error::NoSnapshot { file } => messages.no_snapshot(file),
            Error::Corrupted {
                file,
                seq,
                expected,
                got,
            } => messages.corrupted(file, *seq, *expected, *got),
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for Error {}

/// Результат операции с диском.
pub type Result<T> = std::result::Result<T, Error>;

/// Ошибка `std::io` с путём файла: без пути сообщение бесполезно.
pub(crate) fn io(path: &std::path::Path, reason: std::io::Error) -> Error {
    Error::Read {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

/// То же для записи: ошибки записи нельзя путать с ошибками чтения.
pub(crate) fn write_io(path: &std::path::Path, reason: std::io::Error) -> Error {
    Error::Write {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_message() {
        // Ошибка без текста — это ошибка, которую человек не увидит: молчание
        // хуже, чем неудобное сообщение.
        let errors = [
            Error::Write {
                path: "/tmp/a".to_string(),
                reason: "no space".to_string(),
            },
            Error::Read {
                path: "/tmp/a".to_string(),
                reason: "denied".to_string(),
            },
            Error::PrepareDir {
                path: "/tmp".to_string(),
                reason: "denied".to_string(),
            },
            Error::Rename {
                path: "/tmp/a".to_string(),
                reason: "busy".to_string(),
            },
            Error::Watch {
                path: "/tmp".to_string(),
                reason: "no watch".to_string(),
            },
            Error::WatchFailed {
                reason: "queue is full".to_string(),
            },
            Error::Open {
                path: "/tmp/store.db".to_string(),
                reason: "locked".to_string(),
            },
            Error::Query {
                reason: "locked".to_string(),
            },
            Error::NoSnapshot {
                file: "/tmp/a".to_string(),
            },
        ];
        for error in errors {
            let text = error.text();
            assert!(!text.is_empty(), "{error:?} осталась без текста");
            assert_eq!(format!("{error}"), text);
        }
    }

    #[test]
    fn an_io_error_carries_the_path_it_happened_at() {
        // Ошибка без пути бесполезна: человек не знает, что чинить.
        let path = std::path::Path::new("/tmp/файл.toml");
        let reason = std::io::Error::other("диск кончился");
        match write_io(path, reason) {
            Error::Write { path: at, reason } => {
                assert!(at.contains("файл.toml"), "{at}");
                assert!(reason.contains("диск кончился"), "{reason}");
            }
            other => panic!("не та ошибка: {other:?}"),
        }
    }

    #[test]
    fn read_and_write_errors_do_not_mix() {
        // Запись и чтение чинятся по-разному, значит и говорить о них надо
        // по-разному.
        let path = std::path::Path::new("/tmp/a");
        let reason = std::io::Error::other("x");
        assert!(matches!(io(path, reason), Error::Read { .. }));
        let reason = std::io::Error::other("x");
        assert!(matches!(write_io(path, reason), Error::Write { .. }));
    }
}

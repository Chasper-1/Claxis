use std::fmt;

/// Что пошло не так в хранилище.
#[derive(Debug)]
pub enum Error {
    /// Файл базы не открылся.
    Open { path: String, reason: String },
    /// Каталог для базы не создался.
    PrepareDir { path: String, reason: String },
    /// Запрос к базе не выполнился.
    Query { reason: String },
    /// Снапшота для файла нет.
    NoSnapshot { file: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Open { path, reason } => {
                write!(f, "cannot open store at {path}: {reason}")
            }
            Error::PrepareDir { path, reason } => {
                write!(f, "cannot create store directory {path}: {reason}")
            }
            Error::Query { reason } => write!(f, "store query failed: {reason}"),
            Error::NoSnapshot { file } => write!(f, "no snapshot stored for {file}"),
        }
    }
}

impl std::error::Error for Error {}

/// Результат операции хранилища.
pub type Result<T> = std::result::Result<T, Error>;

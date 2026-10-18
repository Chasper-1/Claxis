/// Что пошло не так.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    /// Диапазон вышел за пределы документа.
    OutOfBounds { anchor: u32, len: u32, doc_len: u32 },
    /// Текст не влезает в общий буфер: его смещения 32-битные.
    BufferTooLarge {
        /// Сколько байт просили положить.
        len: u64,
    },
    /// Пул узлов заполнен: индекс узла 24-битный, и новый узел получил бы
    /// обрезанный номер.
    ///
    /// Правки дробят текст на листья, и упереться в потолок пула можно раньше,
    /// чем в потолок буфера. Узел с обрезанным индексом указал бы не туда
    /// молча, поэтому правка отвергается, не начавшись.
    NodePoolFull {
        /// Сколько слотов пула занято сейчас.
        nodes: usize,
    },
    /// Глубина истории вне диапазона `min..`.
    ///
    /// Потолка нет: история растёт вместе с правками и память добирается по
    /// мере надобности. Отвергается только ноль — с ним снимался бы снапшот
    /// после каждой правки.
    InvalidHistoryDepth { depth: u32 },
    /// В истории лежит не формат rope: прежний буфер писал в ту же колонку
    /// свой блоб, и разбирать его как родной нельзя.
    NotOurHistory,
    /// Версия истории не та, что ждёт крейт.
    UnknownHistoryVersion {
        /// Что пришло.
        version: u8,
    },
    /// История оборвана: данных меньше, чем записано в заголовке.
    TruncatedHistory,
    /// Запись истории неизвестного вида.
    UnknownRecordKind {
        /// Что пришло в байте вида.
        kind: u8,
    },
    /// Запись истории ссылается на кусок буфера, которого нет.
    ///
    /// Смещения приходят из блоба, написанного прошлым запуском. Битый блоб —
    /// это ошибка данных, а не повод ронять редактор вместе с текстом.
    HistoryOutOfBounds {
        /// С какого места буфера просили кусок.
        offset: u32,
        /// Сколько байт просили.
        len: u32,
    },
    /// Запись истории противоречива: диапазон вывернут наоборот.
    ///
    /// Всё то же самое, что битый блоб из базы: данные не наши или испорчены.
    InvalidRecordRange {
        /// Откуда диапазон.
        anchor: u32,
        /// Куда диапазон.
        head: u32,
    },
    /// Снимок, который подтянулся из базы для отката дальше, не разобрался.
    ///
    /// Откат на этом заканчивается: продолжать нечего, а молча остановиться
    /// значит выглядеть так, будто отменять больше нечего.
    OlderSnapshotBroken {
        /// Что именно не разобралось в блобе. Причина едет целиком, а не
        /// готовой английской строкой: текст собирает каталог, и вложенная
        /// ошибка обязана переводиться так же, как внешняя.
        reason: Box<Error>,
    },
}

impl Error {
    /// Текст ошибки на языке каталога. По умолчанию — английский.
    ///
    /// Крейт отдаёт структуру и передаёт каталогу значения. Как они попадут в
    /// текст — дело каталога: в переводе сообщение пишется целиком, с
    /// подстановками в тех местах, куда их поставил переводчик.
    pub fn message(&self, messages: &dyn crate::messages::Messages) -> String {
        match self {
            Error::OutOfBounds {
                anchor,
                len,
                doc_len,
            } => messages.out_of_bounds(*anchor, anchor.saturating_add(*len), *doc_len),
            Error::BufferTooLarge { len } => messages.buffer_too_large(*len),
            Error::NodePoolFull { nodes } => {
                messages.node_pool_full(*nodes, crate::node::MAX_NODES)
            }
            Error::InvalidHistoryDepth { depth } => messages.invalid_history_depth(*depth, 1),
            Error::NotOurHistory => messages.not_our_history(),
            Error::UnknownHistoryVersion { version } => {
                messages.unknown_history_version(*version, crate::history::VERSION)
            }
            Error::TruncatedHistory => messages.truncated_history(),
            Error::UnknownRecordKind { kind } => messages.unknown_record_kind(*kind),
            Error::HistoryOutOfBounds { offset, len } => {
                messages.history_out_of_bounds(*offset, offset.saturating_add(*len))
            }
            Error::InvalidRecordRange { anchor, head } => {
                messages.invalid_record_range(*anchor, *head)
            }
            Error::OlderSnapshotBroken { reason } => {
                messages.older_snapshot_broken(&reason.message(messages))
            }
        }
    }

    /// Текст ошибки на языке по умолчанию.
    pub fn text(&self) -> String {
        self.message(&crate::messages::En)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl std::error::Error for Error {}

/// Результат операции с документом.
pub type Result<T> = std::result::Result<T, Error>;

/// Глубина истории по умолчанию: сколько правок живёт до снапшота.
///
/// Потолка нет: история растёт по мере надобности, а это число только
/// говорит, когда пора снять снапшот и отдать память обратно.
pub const DEFAULT_DEPTH: u32 = 8 * 1024;

/// Минимальная глубина истории.
pub const MIN_DEPTH: u32 = 1;

/// Снимок памяти — для диагностики и тестов.
#[derive(Clone, Copy, Debug)]
pub struct Memory {
    /// Занято узлов.
    pub nodes: usize,
    /// Свободно узлов.
    pub free: usize,
    /// Ёмкость пула.
    pub capacity: usize,
    /// Листьев.
    pub leaves: usize,
    /// Размер узла.
    pub node_size: usize,
    /// Байт в буфере текста.
    pub text_len: usize,
    /// Ёмкость буфера текста.
    pub text_capacity: usize,
    /// Вершин истории.
    pub versions: usize,
    /// Размер вершины.
    pub version_size: usize,
}

impl Memory {
    /// Байт в дереве.
    pub fn tree_bytes(&self) -> usize {
        self.capacity * self.node_size
    }
}

/// Куда уходит снятый снапшот.
pub trait SnapshotSink: std::fmt::Debug {
    /// Документ снят: сохранить текст и историю.
    ///
    /// Ошибка не возвращается: потеря кеша не должна стоить пользователю
    /// работы.
    fn accept(&mut self, path: &std::path::Path, document: &[u8], history: &[u8]);

    /// Блок истории снимка старее того, на котором история в памяти закончилась.
    ///
    /// Вызывается, когда отмена дошла до конца памяти: в ней лежит всего один
    /// снимок, а откатиться надо дальше. Приёмник отдаёт следующий и держит
    /// его в памяти вместо прежнего — так следующая граница не читает диск.
    ///
    /// `None`, когда идти дальше некуда: снимков больше нет.
    fn older(&mut self, path: &std::path::Path) -> Option<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_speak_through_the_catalog() {
        // Ошибка не несёт готовой строки: текст собирает каталог, иначе её нельзя
        // будет перевести.
        let e = Error::OutOfBounds {
            anchor: 10,
            len: 5,
            doc_len: 12,
        };
        let text = e.message(&crate::messages::En);
        assert!(
            text.contains("15"),
            "в тексте должно быть место значения: {text}"
        );
        assert!(
            text.contains("12"),
            "в тексте должно быть место значения: {text}"
        );
        assert_eq!(e.text(), text);
        assert_eq!(format!("{e}"), text);
    }
}

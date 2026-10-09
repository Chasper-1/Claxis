/// Варианты размера арены метаданных.
/// При изменении структуры `Record` или размера арены учитывайте, что
/// ёмкость арены в записях вычисляется как `размер_в_байтах / size_of::<Record>()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaSize {
    Kb128 = 128 * 1024,
    Kb256 = 256 * 1024,
    Kb512 = 512 * 1024,
    Kb768 = 768 * 1024,
    Mb1   = 1024 * 1024,
    Mb2   = 2 * 1024 * 1024,
    Mb4   = 4 * 1024 * 1024,
    Mb6   = 6 * 1024 * 1024,
    Mb8   = 8 * 1024 * 1024,
    Mb10  = 10 * 1024 * 1024,
}

impl ArenaSize {
    pub const fn bytes(self) -> usize {
        self as usize
    }
}

/// Вид изменения текста.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Новое перекрывает диапазон: старое физически цело, лежит под новым.
    Insert,
    /// Диапазон исключён из документа. Текста нет, только границы.
    Delete,
}

/// Запись об изменении — метаданные, без текста. 16 байт.
///
/// **Важно при изменениях арены или записи:**
/// - `Record` имеет фиксированный размер (16 байт при `u32` полях).
/// - При добавлении полей в `Record` размер структуры увеличится, что уменьшит общее
///   количество возможных записей в арене фиксированного размера.
/// - Изменяя логику `AddArena`, следите за согласованностью `SEG_CAPACITY` в `current.rs`,
///   так как емкость сегментов рассчитывается исходя из емкости арены записей.
///
/// `data` указывает, где лежат данные правки, и означает разное по виду правки:
/// для вставки — смещение в растущем буфере вставленного текста, для удаления —
/// смещение отложенных сегментов относительно начала отложенной области.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    kind: Kind,
    pos: u32,
    len: u32,
    data: u32,
}

impl Record {
    pub fn insert(pos: u32, len: u32, text_off: u32) -> Self {
        Self {
            kind: Kind::Insert,
            pos,
            len,
            data: text_off,
        }
    }

    pub fn delete(pos: u32, len: u32, parked: u32) -> Self {
        Self {
            kind: Kind::Delete,
            pos,
            len,
            data: parked,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn pos(&self) -> u32 {
        self.pos
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    /// Смещение данных правки: в растущем буфере для вставки, отложенных
    /// сегментов для удаления.
    pub fn data(&self) -> u32 {
        self.data
    }
}

pub type RecordId = u32;

/// Арена метаданных: append-only, фиксированного размера.
///
/// Текст сюда не попадает — он в растущем буфере. Сюда падают только записи
/// об изменениях: они мелкие, их дохуища, и именно они должны быть ограничены.
#[derive(Debug)]
pub struct AddArena {
    records: Vec<Record>,
    capacity: usize,
}

impl Default for AddArena {
    fn default() -> Self {
        Self::new(ArenaSize::Kb128)
    }
}

impl AddArena {
    pub fn new(size: ArenaSize) -> Self {
        Self {
            records: Vec::new(),
            capacity: size.bytes() / size_of::<Record>(),
        }
    }
    pub fn push(&mut self, record: Record) -> Option<RecordId> {
        if self.records.len() >= self.capacity {
            return None;
        }
        let id = self.records.len() as RecordId;
        self.records.push(record);
        Some(id)
    }

    pub fn get(&self, id: RecordId) -> Option<&Record> {
        self.records.get(id as usize)
    }

    pub fn len(&self) -> u32 {
        self.records.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn remaining(&self) -> u32 {
        (self.capacity - self.records.len()) as u32
    }

    pub fn remaining_records(&self) -> usize {
        self.capacity - self.records.len()
    }

    pub fn remaining_bytes(&self) -> usize {
        self.remaining_records() * size_of::<Record>()
    }

    /// Переиспользование арены: указатель отматывается, записи чистятся.
    /// Память не перевыделяется — записи просто переписываются с нуля.
    pub fn reset(&mut self) {
        self.records.clear();
    }
}

/// Метаданных в арене помещается столько, сколько влезает в 128 КиБ.
/// Снапшот обнуляет историю, а при выходе из редактора она всё равно не нужна,
/// поэтому глубина отмены в 8192 правок никого не ограничивает.
pub const ARENA_CAPACITY: usize = 128 * 1024;

use std::mem::size_of;

/// Размер одного блока арены в байтах. Записи не теряются: когда блок
/// переполняется, выделяется следующий.
///
/// Размеры:
///
/// * от 32КБ до 128КБ — шаг 32КБ;
/// * от 192КБ до 512КБ — шаг 64КБ;
/// * от 640КБ до 1МБ — шаг 128КБ.
///
/// 32КБ — минимальный размер, это 2К записей (запись — 16 байт), для самых
/// экономных. 1МБ — максимум.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaSize {
    Kb32 = 32 * 1024,
    Kb64 = 64 * 1024,
    Kb96 = 96 * 1024,
    Kb128 = 128 * 1024,
    Kb192 = 192 * 1024,
    Kb256 = 256 * 1024,
    Kb320 = 320 * 1024,
    Kb384 = 384 * 1024,
    Kb448 = 448 * 1024,
    Kb512 = 512 * 1024,
    Kb640 = 640 * 1024,
    Kb768 = 768 * 1024,
    Kb896 = 896 * 1024,
    Kb1024 = 1024 * 1024,
}

impl ArenaSize {
    pub const fn bytes(self) -> usize {
        self as usize
    }

    /// Сколько записей помещается в блок этого размера.
    pub const fn records(self) -> usize {
        self.bytes() / size_of::<Record>()
    }
}

/// Запись правки. Операция несётся самой записью — отдельного поля типа нет.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Record {
    /// Вставка. `text` — ссылка на вставленный текст в `Added`.
    Insert { anchor: u32, head: u32, text: u32 },
    /// Удаление. Текста не добавляет, поэтому текста в записи нет.
    Delete { anchor: u32, head: u32 },
}

impl Record {
    pub fn insert(anchor: u32, len: u32, text: u32) -> Self {
        Record::Insert {
            anchor,
            head: anchor + len,
            text,
        }
    }

    pub fn delete(anchor: u32, len: u32) -> Self {
        Record::Delete {
            anchor,
            head: anchor + len,
        }
    }

    /// Якорь — начало диапазона.
    pub fn anchor(&self) -> u32 {
        match self {
            Record::Insert { anchor, .. } | Record::Delete { anchor, .. } => *anchor,
        }
    }

    /// Голова — конец диапазона.
    pub fn head(&self) -> u32 {
        match self {
            Record::Insert { head, .. } | Record::Delete { head, .. } => *head,
        }
    }

    /// Длина диапазона: `anchor - head` по модулю. Знак не важен.
    pub fn len(&self) -> u32 {
        self.anchor().abs_diff(self.head())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ссылка на текст вставки для `Insert`.
    pub fn text(&self) -> Option<u32> {
        match self {
            Record::Insert { text, .. } => Some(*text),
            Record::Delete { .. } => None,
        }
    }
}

/// Идентификатор записи.
pub type RecordId = u32;

/// Зарезервированный идентификатор: сегмент без записи — текст `Original`.
pub const ORIGINAL_ID: RecordId = 0;

/// Арена записей правок.
///
/// Записи лежат в блоках фиксированного размера. Когда текущий блок
/// переполняется, под записи выделяется следующий — записи не теряются и не
/// переприсваиваются. Идентификатор записи остаётся стабильным, на него
/// ссылаются сегменты.
///
/// Внутри живут два стека: в `undo` пишется всегда и всё, в `redo` живут
/// только отменённые записи.
#[derive(Debug)]
pub struct Arena {
    /// Блоки записей. Каждый имеет ёмкость записи в `capacity`.
    blocks: Vec<Vec<Record>>,
    /// Ёмкость одного блока в записях.
    capacity: usize,
    undo: Vec<RecordId>,
    redo: Vec<RecordId>,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new(ArenaSize::Kb128)
    }
}

impl Arena {
    pub fn new(size: ArenaSize) -> Self {
        let capacity = size.records();
        // Первый слот первого блока зарезервирован под Original (ORIGINAL_ID)
        // — под него кладётся пустышка, и он не используется.
        let mut blocks = vec![Vec::with_capacity(capacity)];
        blocks[0].push(Record::Delete { anchor: 0, head: 0 });
        Self {
            blocks,
            capacity,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Кладёт запись в текущий блок (новый, если тот полон) и в `undo`.
    pub fn push(&mut self, record: Record) -> RecordId {
        let last = self.blocks.last().unwrap();
        if last.len() >= self.capacity {
            self.blocks.push(Vec::with_capacity(self.capacity));
        }
        let block = self.blocks.len() - 1;
        let slot = self.blocks[block].len();
        self.blocks[block].push(record);
        let id = block as RecordId * self.capacity as RecordId + slot as RecordId;
        self.undo.push(id);
        id
    }

    pub fn get(&self, id: RecordId) -> Option<&Record> {
        if id == ORIGINAL_ID {
            return None;
        }
        let block = (id / self.capacity as RecordId) as usize;
        let slot = (id % self.capacity as RecordId) as usize;
        self.blocks.get(block)?.get(slot)
    }

    /// Сколько записей записано (без зарезервированного нулевого слота).
    pub fn len(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum::<usize>() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Свободных мест в текущем блоке.
    pub fn remaining_records(&self) -> usize {
        self.capacity - self.blocks.last().unwrap().len()
    }

    /// Ёмкость одного блока в записях.
    pub fn capacity_records(&self) -> usize {
        self.capacity
    }

    /// Сколько блоков выделено.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Ёмкость пула записанных записей.
    pub fn record_capacity(&self) -> usize {
        self.blocks.iter().map(Vec::capacity).sum()
    }

    /// Верхняя запись `undo` переезжает в `redo`.
    pub fn undo(&mut self) -> Option<RecordId> {
        let id = self.undo.pop()?;
        self.redo.push(id);
        Some(id)
    }

    /// Верхняя запись `redo` возвращается в `undo`.
    pub fn redo(&mut self) -> Option<RecordId> {
        let id = self.redo.pop()?;
        self.undo.push(id);
        Some(id)
    }

    /// Новая правка отменяет redo-ветку. Возвращает очищенные идентификаторы.
    pub fn discard_redo(&mut self) -> Vec<RecordId> {
        std::mem::take(&mut self.redo)
    }

    pub fn undo_stack(&self) -> &[RecordId] {
        &self.undo
    }

    pub fn redo_stack(&self) -> &[RecordId] {
        &self.redo
    }

    /// Полный сброс после снапшота: блоки и стеки очищаются.
    pub fn reset(&mut self) {
        self.blocks.clear();
        self.blocks.push(Vec::with_capacity(self.capacity));
        self.blocks[0].push(Record::Delete { anchor: 0, head: 0 });
        self.undo.clear();
        self.redo.clear();
    }
}

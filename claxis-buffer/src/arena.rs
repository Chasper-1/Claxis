/// Максимальная глубина истории в записях — 128 КБ, 8192 записей по 16 байт.
///
/// Выбор размера арены отдельной настройкой не делается: держать больше 8192
/// записей незачем. Всё, что не помещается, уходит на диск снапшотами (§11).
pub const MAX_DEPTH: u32 = 8 * 1024;

/// Глубина истории по умолчанию: полный блок, 128 КБ.
pub const DEFAULT_DEPTH: u32 = MAX_DEPTH;

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
        Self::new(DEFAULT_DEPTH)
    }
}

impl Arena {
    /// Арена глубиной в `depth` записей.
    ///
    /// Глубина — сколько записей живёт в памяти до снапшота. Проверку диапазона
    /// делает вызывающий: буфер возвращает ошибку, а не паникует, потому что
    /// значение приходит из конфига.
    pub fn new(depth: u32) -> Self {
        debug_assert!((1..=MAX_DEPTH).contains(&depth));
        let capacity = depth as usize;
        // Первый слот первого блока зарезервирован под Original (ORIGINAL_ID)
        // — под него кладётся пустышка, и он не используется.
        let mut blocks = vec![Vec::with_capacity(capacity)];
        blocks[0].push(Record::Delete { anchor: 0, head: 0 });
        Self {
            blocks,
            capacity,
            undo: Vec::with_capacity(capacity),
            redo: Vec::new(),
        }
    }

    /// Глубина истории в записях — сколько записей помещается без снапшота.
    pub fn depth(&self) -> usize {
        self.capacity
    }

    /// Кладёт запись в текущий блок (новый, если тот полон) и в `undo`.
    ///
    /// Блоков в арене всегда хотя бы один: он создаётся в `new` и `reset`
    /// ничего не удаляет насовсем. Поэтому обращения идут через `first_mut`
    /// и `last_mut`, без паники на «должно существовать».
    pub fn push(&mut self, record: Record) -> RecordId {
        if self
            .blocks
            .last_mut()
            .is_none_or(|last| last.len() >= self.capacity)
        {
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
        let used = self.blocks.last().map_or(0, Vec::len);
        self.capacity - used
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

    /// Очистить redo-ветку на месте, без перемещения вектора.
    ///
    /// Ёмкость стека сохраняется и используется следующей веткой отмены —
    /// освобождать и потом запрашивать память заново незачем.
    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    pub fn undo_stack(&self) -> &[RecordId] {
        &self.undo
    }

    pub fn redo_stack(&self) -> &[RecordId] {
        &self.redo
    }

    /// Полный сброс после снапшота: блоки и стеки очищаются.
    ///
    /// Первый блок **переиспользуется**: он держит в себе буфер на
    /// `capacity` записей, и выбрасывать его с последующим новым выделением
    /// незачем — снапшот берётся часто, а лишние `free`/`malloc` на 128КБ
    /// ничего не дают. Дополнительные блоки освобождаются: держать их память
    /// после схлопывания незачем.
    pub fn reset(&mut self) {
        self.blocks.truncate(1);
        let first = &mut self.blocks[0];
        first.clear();
        first.push(Record::Delete { anchor: 0, head: 0 });
        self.undo.clear();
        self.redo.clear();
    }
}

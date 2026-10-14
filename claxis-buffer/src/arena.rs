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
    /// Удаление. `text` — ссылка на удалённый текст в `Removed`.
    ///
    /// Раньше текста в записи не было: удалённые байты лежали на месте, но
    /// ссылок на них в дереве уже не оставалось. Теперь отмена удаления
    /// обходится одной операцией, и для этого нужно помнить, что именно было
    /// вырезано. Платит ровно те же 16 байт: третье поле уже было.
    Delete { anchor: u32, head: u32, text: u32 },
}

impl Record {
    pub fn insert(anchor: u32, len: u32, text: u32) -> Self {
        Record::Insert {
            anchor,
            head: anchor + len,
            text,
        }
    }

    pub fn delete(anchor: u32, len: u32, text: u32) -> Self {
        Record::Delete {
            anchor,
            head: anchor + len,
            text,
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

    /// Ссылка на текст: в `Added` для вставки, в `Removed` для удаления.
    pub fn text(&self) -> Option<u32> {
        match self {
            Record::Insert { text, .. } | Record::Delete { text, .. } => Some(*text),
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
    /// Вершины истории, по индексу — ссылка на вершину.
    ///
    /// Вершина несёт запись правки и две ссылки: на родителя и на ребёнка.
    /// Стеков нет: отмена — шаг к родителю, повтор — шаг к ребёнку. Два стека
    /// занимали бы столько же памяти, но теряли путь, откуда пришли.
    nodes: Vec<Node>,
    /// Свободные слоты: индексы вершин, которые больше не нужны.
    free: Vec<RecordId>,
    /// Вершина, в которой мы находимся: её запись — последняя применённая.
    current: RecordId,
    /// Ёмкость одного блока вершин.
    capacity: usize,
}

/// Вершина истории.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Запись правки, которой вершина достигнута.
    record: Record,
    /// Предыдущая вершина. `None` — корень, до первой правки.
    parent: Option<RecordId>,
    /// Следующая вершина: куда возвращает повтор.
    child: Option<RecordId>,
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
        let mut nodes = Vec::with_capacity(capacity);
        // Корень без записи: до первой правки отменять нечего. Он же занимает
        // слот 0, поэтому первая правка получает идентификатор 1.
        nodes.push(Node {
            record: Record::delete(0, 0, 0),
            parent: None,
            child: None,
        });
        Self {
            nodes,
            free: Vec::new(),
            current: ORIGINAL_ID,
            capacity,
        }
    }

    /// Глубина истории в записях — сколько записей помещается без снапшота.
    pub fn depth(&self) -> usize {
        self.capacity
    }

    /// Сделать запись текущей вершиной истории.
    ///
    /// Новая правка отбрасывает redo-ветку: ребёнок текущей вершины
    /// заменяется, и отброшенная ветка становится недостижимой из корня.
    pub fn push(&mut self, record: Record) -> RecordId {
        let id = self.alloc();
        let parent = self.current;
        self.nodes[id as usize] = Node {
            record,
            parent: Some(parent),
            child: None,
        };
        self.nodes[parent as usize].child = Some(id);
        self.current = id;
        id
    }

    /// Запись вершины.
    pub fn get(&self, id: RecordId) -> Option<&Record> {
        if id == ORIGINAL_ID {
            return None;
        }
        self.nodes.get(id as usize).map(|n| &n.record)
    }

    /// Вершина, в которой мы находимся.
    pub fn current(&self) -> RecordId {
        self.current
    }

    /// Шаг назад: запись текущей вершины отменяется, курсор идёт к родителю.
    pub fn undo(&mut self) -> Option<RecordId> {
        let id = self.current;
        let parent = self.nodes[id as usize].parent?;
        self.current = parent;
        Some(id)
    }

    /// Шаг вперёд: возвращаемся к ребёнку и применяем его запись.
    pub fn redo(&mut self) -> Option<RecordId> {
        let child = self.nodes[self.current as usize].child?;
        self.current = child;
        Some(child)
    }

    /// Отбросить redo-ветку и обрезать `Removed` по хвосту.
    ///
    /// Отброшенная ветка — это путь от текущей вершины по детям. Тексты
    /// удалений, которые она накопила, больше никому не понадобятся, и
    /// хвост достаточно срезать: ни удаления, ни перемещения памяти.
    pub fn discard_child(&mut self) -> Option<u32> {
        let mut tail = None;
        let mut cursor = self.nodes[self.current as usize].child;
        while let Some(id) = cursor {
            let node = self.nodes[id as usize];
            // Текст держат обе записи: вставка кладёт свой, удаление —
            // вырезанный. Хвост срезается по наименьшему из них.
            if let Some(text) = node.record.text() {
                tail = Some(tail.map_or(text, |t: u32| t.min(text)));
            }
            // Отрезаем ветку на ходу: дальше по ней не пойдут.
            self.release(id);
            cursor = node.child;
        }
        self.nodes[self.current as usize].child = None;
        tail
    }

    /// Отпустить слот вершины в свободные.
    fn release(&mut self, id: RecordId) {
        self.free.push(id);
    }

    /// Взять свободный слот или выделить новый.
    fn alloc(&mut self) -> RecordId {
        match self.free.pop() {
            Some(id) => id,
            None => {
                self.nodes.push(Node {
                    record: Record::delete(0, 0, 0),
                    parent: None,
                    child: None,
                });
                (self.nodes.len() - 1) as RecordId
            }
        }
    }

    /// Сколько записей записано (без корня).
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Сколько записей ещё поместится без снапшота.
    ///
    /// Считаются живые вершины, а не занятые слоты: отброшенная redo-ветка
    /// места не занимает, её слоты уже в свободном списке.
    pub fn remaining_records(&self) -> usize {
        // Живых вершин столько, сколько занято слотов минус свободные:
        // отброшенная redo-ветка места не занимает.
        let live = self.nodes.len() - self.free.len();
        self.capacity.saturating_sub(live)
    }

    /// Ёмкость одного блока в записях.
    pub fn capacity_records(&self) -> usize {
        self.capacity
    }

    /// Сколько блоков выделено.
    pub fn block_count(&self) -> usize {
        self.nodes.len().div_ceil(self.capacity).max(1)
    }

    /// Ёмкость пула записанных записей.
    pub fn record_capacity(&self) -> usize {
        self.nodes.capacity()
    }

    /// Путь от текущей вершины по детям: отброшенная redo-ветка.
    pub fn redo_path(&self) -> Vec<RecordId> {
        let mut out = Vec::new();
        let mut cursor = self.nodes[self.current as usize].child;
        while let Some(id) = cursor {
            out.push(id);
            cursor = self.nodes.get(id as usize).and_then(|n| n.child);
        }
        out
    }

    /// Отладочный снимок вершин: для тестов дерева истории.
    pub fn debug_nodes(&self) -> Vec<(Option<RecordId>, Option<RecordId>, bool)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.parent, n.child, i as RecordId == self.current))
            .collect()
    }

    /// Путь от корня до текущей вершины: записи по порядку применения.
    ///
    /// Нужен для пересборки дерева сегментов и для сохранения снапшота.
    pub fn path(&self) -> Vec<RecordId> {
        let mut out = Vec::with_capacity(self.len());
        let mut cursor = self.current;
        while cursor != ORIGINAL_ID {
            out.push(cursor);
            cursor = match self.nodes.get(cursor as usize).and_then(|n| n.parent) {
                Some(p) => p,
                None => break,
            };
        }
        out.reverse();
        out
    }

    /// Заменить историю целиком: восстановление после снапшота.
    ///
    /// Записи приходят уже разобранными и упорядоченными по времени, поэтому
    /// вершины просто идут подряд и связываются родителем. Идентификаторы
    /// совпадают с порядком: этого требует `Record`, в которой ссылки на
    /// записи записаны как есть.
    pub fn restore(&mut self, undo: Vec<Record>, redo: Vec<Record>) {
        self.reset();
        for record in undo {
            self.push(record);
        }
        // Отброшенная ветка вешается на текущую вершину: повтор после
        // перезапуска должен работать так же, как до него.
        let mut cursor = self.current;
        for record in redo {
            let id = self.alloc();
            self.nodes[id as usize] = Node {
                record,
                parent: Some(cursor),
                child: None,
            };
            self.nodes[cursor as usize].child = Some(id);
            cursor = id;
        }
    }

    /// Полный сброс после снапшота: история начинается заново.
    pub fn reset(&mut self) {
        self.nodes.truncate(1);
        self.nodes[0] = Node {
            record: Record::delete(0, 0, 0),
            parent: None,
            child: None,
        };
        self.free.clear();
        self.current = ORIGINAL_ID;
    }
}

use std::mem::size_of;

use crate::arena::ORIGINAL_ID;
use crate::arena::Record;
use crate::segment::{ORIGINAL, Segment};

/// Узел дерева сегментов: лист-сегмент или ветка с двумя детьми.
///
/// Ровно 16 байт. Обычный `enum` стоил бы 24: восемь байт уходили бы на
/// дискриминант, который здесь не нужен — вид узла и так определяется по
/// старшему биту поля `a`. Индексы узлов и записи меньше `2^31`, поэтому бит
/// свободен.
///
/// У ветки кэшируются длина поддерева и высота — по ним идёт спуск к позиции
/// за O(log n), без обхода сегментов.
#[derive(Clone, Copy)]
#[repr(C)]
struct Node {
    /// Ветка: индекс левого ребёнка. Лист: `LEAF | record`.
    a: u32,
    /// Ветка: индекс правого ребёнка. Лист: позиция.
    b: u32,
    /// Ветка: длина поддерева. Лист: длина текста.
    len: u32,
    /// Ветка: высота поддерева. Лист: смещение в источнике.
    c: u32,
}

/// Старший бит `a`: узел-лист. Узлы-ветки держат в `a` индекс ребёнка.
const LEAF: u32 = 1 << 31;

/// Узел свободен: поле листа обнулено, чтобы не держать старый текст.
const DEAD: Node = Node {
    a: LEAF,
    b: 0,
    len: 0,
    c: 0,
};

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_leaf() {
            write!(f, "Leaf {:?}", self.leaf_seg())
        } else {
            write!(
                f,
                "Branch {{ left: {}, right: {}, len: {}, height: {} }}",
                self.left(),
                self.right(),
                self.len,
                self.height()
            )
        }
    }
}

impl Node {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.a & LEAF != 0
    }

    #[inline]
    fn leaf_seg(&self) -> Segment {
        Segment {
            record: self.a & !LEAF,
            pos: self.b,
            len: self.len,
            off: self.c,
        }
    }

    #[inline]
    fn left(&self) -> u32 {
        self.a
    }

    #[inline]
    fn right(&self) -> u32 {
        self.b
    }

    #[inline]
    fn height(&self) -> u16 {
        self.c as u16
    }

    fn leaf(seg: Segment) -> Node {
        Node {
            a: LEAF | seg.record,
            b: seg.pos,
            len: seg.len,
            c: seg.off,
        }
    }

    fn branch(left: u32, right: u32, len: u32, height: u16) -> Node {
        Node {
            a: left,
            b: right,
            len,
            c: height as u32,
        }
    }
}

/// Пустое поддерево — пустой документ.
const NIL: u32 = u32::MAX;

/// Источники байт на время чтения.
struct ReadCtx<'a> {
    original: &'a [u8],
    added: &'a [u8],
}

impl ReadCtx<'_> {
    /// Срез источника: `src` — `ORIGINAL` или `ADDED`.
    fn slice(&self, src: u8, off: u32, len: u32) -> &[u8] {
        let off = off as usize;
        let len = len as usize;
        let source = if src == ORIGINAL {
            self.original
        } else {
            self.added
        };
        source.get(off..off + len).unwrap_or_default()
    }
}

/// Потолок числа сегментов в дереве. Достигнут — берётся снапшот (§11).
pub const MAX_LEAVES: u32 = 16 * 1024;

/// Ёмкость пула узлов.
///
/// Дерево полное двоичное: при `n` листьях внутренних узлов ровно `n − 1`,
/// всего `2n − 1`. Плюс запас на несколько узлов, которые успевают появиться
/// между правкой и проверкой порога.
const NODE_CAPACITY: usize = 2 * MAX_LEAVES as usize + 8;

/// `Current` — кеш: дерево сегментов в порядке документа.
///
/// Каждый сегмент несёт живой текст в порядке документа. Байтов не хранит:
/// текст лежит в `Original` и `Added`, сегменты описывают, откуда и сколько
/// взять. Удаление вырезает сегменты из дерева — мёртвых сегментов не бывает,
/// при чтении смотреть на метки не надо.
///
/// Дерево сбалансировано по высоте. Вставка и удаление не двигают остальные
/// сегменты: меняется только путь от корня к точке правки, поэтому обе
/// операции стоят O(log n) независимо от размера документа.
///
/// Пул узлов выделяется **один раз** на весь срок жизни и больше не растёт:
/// ёмкость известна из потолка листьев, а список свободных узлов хранится в
/// самих освобождённых узлах. В горячем пути аллокаций нет вообще.
#[derive(Debug)]
pub struct Current {
    /// Пул узлов фиксированного размера. Индекс узла — его ссылка.
    nodes: Vec<Node>,
    /// Корень дерева.
    root: u32,
    /// Длина документа в байтах.
    len: u32,
    /// Число живых сегментов (листьев).
    leaves: u32,
    /// Голова списка свободных узлов. Список хранится в самих узлах.
    free_head: u32,
    /// Следующий слот, который ещё ни разу не выдавался.
    next_unused: u32,
}

impl Current {
    /// Единое состояние: один лист на весь исходный текст.
    pub fn from_original(original_len: u32) -> Self {
        let mut cur = Self {
            nodes: vec![DEAD; NODE_CAPACITY],
            root: NIL,
            len: 0,
            leaves: 0,
            free_head: NIL,
            next_unused: 0,
        };
        if original_len > 0 {
            let leaf = cur.make_leaf(Segment {
                record: ORIGINAL_ID,
                pos: 0,
                len: original_len,
                off: 0,
            });
            cur.root = leaf;
            cur.leaves = 1;
            cur.len = original_len;
        }
        cur
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Число сегментов в дереве.
    pub fn seg_count(&self) -> u32 {
        self.leaves
    }

    /// Высота дерева: 0 — пусто, 1 — один лист.
    pub fn height(&self) -> usize {
        self.height_of(self.root)
    }

    /// Занятых узлов — тех, что сейчас входят в дерево.
    pub fn node_count(&self) -> usize {
        let mut chained = 0;
        let mut idx = self.free_head;
        while idx != NIL {
            chained += 1;
            idx = self.nodes[idx as usize].a;
        }
        self.next_unused as usize - chained
    }

    /// Мест в пуле под узлы.
    pub fn node_capacity(&self) -> usize {
        self.nodes.len()
    }

    /// Свободных узлов в пуле.
    pub fn free_count(&self) -> usize {
        let mut n = 0;
        let mut idx = self.free_head;
        while idx != NIL {
            n += 1;
            idx = self.nodes[idx as usize].a;
        }
        self.nodes.len() - self.next_unused as usize + n
    }

    /// Ёмкость пула узлов — она постоянна.
    pub const fn node_capacity_fixed() -> usize {
        NODE_CAPACITY
    }

    /// Размер узла в байтах.
    pub const fn node_size() -> usize {
        size_of::<Node>()
    }

    /// Размер сегмента в байтах.
    pub const fn segment_size() -> usize {
        size_of::<Segment>()
    }

    // ── пул узлов ────────────────────────────────────────────────────────

    /// Взять узел. Сначала переиспользуем освобождённые, иначе берём
    /// следующий нетронутый слот. Аллокатор не участвует никогда.
    ///
    /// Пул не кончается: занято не больше `2 · MAX_LEAVES − 1` узлов при
    /// ёмкости `2 · MAX_LEAVES + 8`.
    fn alloc(&mut self, node: Node) -> u32 {
        let idx = if self.free_head != NIL {
            let idx = self.free_head;
            self.free_head = self.nodes[idx as usize].a;
            idx
        } else {
            let idx = self.next_unused;
            assert!(
                (idx as usize) < self.nodes.len(),
                "пул узлов исчерпан: дерево выросло сверх потолка"
            );
            self.next_unused += 1;
            idx
        };
        self.nodes[idx as usize] = node;
        idx
    }

    fn make_leaf(&mut self, seg: Segment) -> u32 {
        self.leaves += 1;
        self.alloc(Node::leaf(seg))
    }

    /// Вернуть один узел в пул. Поддерево не освобождается — только узел.
    ///
    /// Список свободных хранится в самих освобождённых узлах: в поле `a`
    /// лежит индекс следующего. Отдельного векла нет.
    fn release(&mut self, idx: u32) {
        self.nodes[idx as usize] = Node {
            a: self.free_head,
            b: 0,
            len: 0,
            c: 0,
        };
        self.free_head = idx;
    }

    /// Вернуть пул в исходное состояние за O(1).
    ///
    /// Содержимое узлов переписывать не нужно: нетронутые слоты и так
    /// свободны, а слоты прежней цепочки будут перезаписаны при выделении.
    /// Так снятие снапшота не проходит линейно по всему пулу.
    fn reset_pool(&mut self) {
        self.free_head = NIL;
        self.next_unused = 0;
    }

    /// Вернуть узел со всем поддеревом в пул.
    fn release_tree(&mut self, idx: u32) {
        if idx == NIL {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            self.leaves -= 1;
        } else {
            self.release_tree(node.left());
            self.release_tree(node.right());
        }
        self.release(idx);
    }

    // ── свойства узла ─────────────────────────────────────────────────────

    fn len_of(&self, idx: u32) -> u32 {
        if idx == NIL {
            return 0;
        }
        self.nodes[idx as usize].len
    }

    fn height_of(&self, idx: u32) -> usize {
        if idx == NIL {
            return 0;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            1
        } else {
            node.height() as usize
        }
    }

    fn new_branch(&mut self, left: u32, right: u32) -> u32 {
        let len = self.len_of(left) + self.len_of(right);
        let height = 1 + self.height_of(left).max(self.height_of(right)) as u16;
        self.alloc(Node::branch(left, right, len, height))
    }

    /// Склеить два поддерева в одно сбалансированное.
    ///
    /// Высоты могут расходиться на сколько угодно — так бывает при удалении,
    /// когда одно поддерево схлопнулось вместе с вырезанным куском.
    ///
    /// Способ один: спускаемся по краю высокого поддерева, пока оно выше
    /// низкого больше чем на единицу, и приклеиваем низкое к краю. Если
    /// высоты встретились ровно — просто ветка. Если же низкое поддерево
    /// переросло оставшийся край (точка расхождения), оно разрезается
    /// пополам и обе половины приклеиваются к двум сторонам.
    ///
    /// Рекурсия идёт только по краю: высота аргументов на каждом шаге строго
    /// меньше, поэтому выход гарантирован. Склеивать обе стороны сразу нельзя
    /// — восстановленный узел получает ту же высоту, что был, и рекурсия
    /// закручивается.
    fn join(&mut self, a: u32, b: u32) -> u32 {
        if a == NIL {
            return b;
        }
        if b == NIL {
            return a;
        }
        let (ha, hb) = (self.height_of(a), self.height_of(b));
        if ha > hb + 1 {
            let na = self.nodes[a as usize];
            debug_assert!(!na.is_leaf());
            let (al, ar) = (na.left(), na.right());
            if self.height_of(ar) >= hb {
                // Края хватает: приклеиваем b к правому краю a.
                let new_ar = self.join(ar, b);
                self.release(a);
                self.new_branch(al, new_ar)
            } else {
                // Точка расхождения: режем b пополам.
                let nb = self.nodes[b as usize];
                debug_assert!(!nb.is_leaf());
                let (bl, br) = (nb.left(), nb.right());
                let new_l = self.join(ar, bl);
                let mid = self.new_branch(new_l, br);
                self.release(b);
                let out = self.new_branch(al, mid);
                self.release(a);
                out
            }
        } else if hb > ha + 1 {
            let nb = self.nodes[b as usize];
            debug_assert!(!nb.is_leaf());
            let (bl, br) = (nb.left(), nb.right());
            if self.height_of(bl) >= ha {
                let new_bl = self.join(a, bl);
                self.release(b);
                self.new_branch(new_bl, br)
            } else {
                let na = self.nodes[a as usize];
                debug_assert!(!na.is_leaf());
                let (al, ar) = (na.left(), na.right());
                let new_r = self.join(ar, bl);
                let mid = self.new_branch(al, new_r);
                self.release(a);
                let out = self.new_branch(mid, br);
                self.release(b);
                out
            }
        } else {
            self.new_branch(a, b)
        }
    }

    // ── вставка ───────────────────────────────────────────────────────────

    /// Вставка `ins_len` байт из `Added` начиная с `text_off` на позицию `pos`.
    pub(crate) fn apply_insert(&mut self, pos: u32, text_off: u32, ins_len: u32, id: u32) {
        let leaf = self.make_leaf(Segment {
            record: id,
            pos,
            len: ins_len,
            off: text_off,
        });
        self.root = self.insert_rec(self.root, 0, pos, leaf);
        self.len += ins_len;
    }

    /// Лист `new_leaf` вставляется на позицию `pos` внутри поддерева `idx`,
    /// лежащего от документа от `base`. Возвращает новое поддерево.
    fn insert_rec(&mut self, idx: u32, base: u32, pos: u32, new_leaf: u32) -> u32 {
        if idx == NIL {
            return new_leaf;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let seg = node.leaf_seg();
            let off = pos - base;
            if off == 0 {
                return self.join(new_leaf, idx);
            }
            if off >= seg.len {
                return self.join(idx, new_leaf);
            }
            // Разрез листа: голова | вставка | хвост.
            let head = self.make_leaf(Segment {
                record: seg.record,
                pos: seg.pos,
                len: off,
                off: seg.off,
            });
            let tail = self.make_leaf(Segment {
                record: seg.record,
                pos: seg.pos + off,
                len: seg.len - off,
                off: seg.off + off,
            });
            self.leaves -= 1; // старый лист разрезан на два
            self.release(idx);
            let mid = self.join(new_leaf, tail);
            self.join(head, mid)
        } else {
            let (left, right) = (node.left(), node.right());
            let left_len = self.len_of(left);
            let (nl, nr) = if pos < base + left_len {
                (self.insert_rec(left, base, pos, new_leaf), right)
            } else {
                (left, self.insert_rec(right, base + left_len, pos, new_leaf))
            };
            self.release(idx);
            self.join(nl, nr)
        }
    }

    // ── удаление ──────────────────────────────────────────────────────────

    /// Физическое удаление диапазона `pos..pos+len` из дерева.
    ///
    /// Граничные листья обрезаются по границе диапазона, полностью покрытые
    /// поддеревья уходят в пул. Байты никуда не исчезают: они остаются в
    /// `Original` и `Added`, из `Current` уходит только ссылка.
    pub(crate) fn apply_delete(&mut self, pos: u32, len: u32) {
        if len == 0 || self.root == NIL {
            return;
        }
        self.root = self.delete_rec(self.root, 0, pos, pos + len);
        self.len -= len;
    }

    /// Вырезает `start..end` из поддерева `idx`, лежащего от `base`.
    fn delete_rec(&mut self, idx: u32, base: u32, start: u32, end: u32) -> u32 {
        if idx == NIL {
            return NIL;
        }
        let node_end = base + self.len_of(idx);
        // Нет пересечения — поддерево остаётся как есть.
        if end <= base || start >= node_end {
            return idx;
        }
        // Полное покрытие — поддерево уходит целиком.
        if start <= base && end >= node_end {
            self.release_tree(idx);
            return NIL;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let seg = node.leaf_seg();
            // Частичное покрытие листа: остаётся то, что вне диапазона.
            let off_start = start.saturating_sub(base);
            let off_end = (end - base).min(seg.len);
            let head = if off_start > 0 {
                self.make_leaf(Segment {
                    record: seg.record,
                    pos: seg.pos,
                    len: off_start,
                    off: seg.off,
                })
            } else {
                NIL
            };
            let tail = if off_end < seg.len {
                self.make_leaf(Segment {
                    record: seg.record,
                    pos: seg.pos + off_end,
                    len: seg.len - off_end,
                    off: seg.off + off_end,
                })
            } else {
                NIL
            };
            self.leaves -= 1;
            self.release(idx);
            self.join(head, tail)
        } else {
            let (left, right) = (node.left(), node.right());
            // База правого ребёнка — конец левого. Считаем до рекурсии:
            // после спуска `left` может быть уже освобождён.
            let right_base = base + self.len_of(left);
            let nl = self.delete_rec(left, base, start, end);
            let nr = self.delete_rec(right, right_base, start, end);
            self.release(idx);
            self.join(nl, nr)
        }
    }

    /// Пересборка с нуля: один лист на `Original`, затем записи по порядку.
    pub(crate) fn rebuild(
        &mut self,
        original_len: u32,
        records: impl Iterator<Item = (u32, Record)>,
    ) {
        self.clear();
        if original_len > 0 {
            let leaf = self.make_leaf(Segment {
                record: ORIGINAL_ID,
                pos: 0,
                len: original_len,
                off: 0,
            });
            self.root = leaf;
            self.leaves = 1;
            self.len = original_len;
        }
        for (id, record) in records {
            match record {
                Record::Insert { anchor, head, text } => {
                    self.apply_insert(anchor, text, head - anchor, id);
                }
                Record::Delete { anchor, head } => {
                    let len = head - anchor;
                    if len > 0 {
                        self.apply_delete(anchor, len);
                    }
                }
            }
        }
    }

    fn clear(&mut self) {
        self.root = NIL;
        self.len = 0;
        self.leaves = 0;
        self.reset_pool();
    }

    /// Сбросить дерево до одного листа на весь документ, **не перевыделяя пул**.
    ///
    /// Так снятие снапшота не трогает аллокатор: пул остаётся тот же самый,
    /// просто всё дерево схлопывается в один лист за O(1) плюс один узел.
    pub(crate) fn reset_to_single_leaf(&mut self, seg: Segment) {
        self.clear();
        self.root = self.make_leaf(seg);
        self.leaves = 1;
        self.len = seg.len;
    }

    // ── чтение ────────────────────────────────────────────────────────────

    pub fn read(&self, original: &[u8], added: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len as usize);
        self.read_into(&mut out, original, added);
        out
    }

    pub fn read_into(&self, out: &mut Vec<u8>, original: &[u8], added: &[u8]) {
        let ctx = ReadCtx { original, added };
        self.read_all_rec(self.root, &ctx, out);
    }

    fn read_all_rec(&self, idx: u32, ctx: &ReadCtx<'_>, out: &mut Vec<u8>) {
        if idx == NIL {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let seg = node.leaf_seg();
            out.extend_from_slice(ctx.slice(seg.src(), seg.off, seg.len));
        } else {
            self.read_all_rec(node.left(), ctx, out);
            self.read_all_rec(node.right(), ctx, out);
        }
    }

    /// Частичное чтение диапазона: обход с отсечением по длине, O(log n + ответ).
    pub fn read_range(&self, start: u32, end: u32, original: &[u8], added: &[u8]) -> Vec<u8> {
        let start = start.min(self.len);
        let end = end.min(self.len).max(start);
        let mut out = Vec::new();
        if start < end {
            let ctx = ReadCtx { original, added };
            self.read_range_rec(self.root, 0, start, end, &ctx, &mut out);
        }
        out
    }

    fn read_range_rec(
        &self,
        idx: u32,
        base: u32,
        start: u32,
        end: u32,
        ctx: &ReadCtx<'_>,
        out: &mut Vec<u8>,
    ) {
        if idx == NIL {
            return;
        }
        let node_end = base + self.len_of(idx);
        if node_end <= start || base >= end {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let seg = node.leaf_seg();
            let a = start.saturating_sub(base).min(seg.len);
            let b = (end - base).min(seg.len);
            if b > a {
                out.extend_from_slice(ctx.slice(seg.src(), seg.off + a, b - a));
            }
        } else {
            let (left, right) = (node.left(), node.right());
            // База правого ребёнка — конец левого, а не конец всего
            // поддерева: иначе рекурсия уйдёт вправо слишком далеко.
            let right_base = base + self.len_of(left);
            self.read_range_rec(left, base, start, end, ctx, out);
            self.read_range_rec(right, right_base, start, end, ctx, out);
        }
    }

    /// Отладочная проверка: нет ли в дереве повторных индексов, какова
    /// настоящая глубина и не указывает ли живой узел на освобождённый.
    #[cfg(test)]
    pub fn debug_walk(&self) -> (usize, usize, Option<u32>) {
        let mut is_free = vec![false; self.nodes.len()];
        let mut free = self.free_head;
        while free != NIL {
            is_free[free as usize] = true;
            free = self.nodes[free as usize].a;
        }
        let mut seen = vec![false; self.nodes.len()];
        let mut count = 0usize;
        let mut depth = 0usize;
        let mut cycle = None;
        let mut stack = vec![(self.root, 1usize)];
        while let Some((idx, d)) = stack.pop() {
            if idx == NIL {
                continue;
            }
            if seen[idx as usize] {
                cycle = Some(idx);
                break;
            }
            if is_free[idx as usize] {
                panic!("живой узел {idx} оказался в свободном списке");
            }
            seen[idx as usize] = true;
            count += 1;
            depth = depth.max(d);
            let node = self.nodes[idx as usize];
            if !node.is_leaf() {
                stack.push((node.left(), d + 1));
                stack.push((node.right(), d + 1));
            }
        }
        (count, depth, cycle)
    }

    /// Все сегменты в порядке документа — для тестов и отладки.
    pub fn segments(&self, out: &mut Vec<Segment>) {
        out.clear();
        self.collect(self.root, out);
    }

    fn collect(&self, idx: u32, out: &mut Vec<Segment>) {
        if idx == NIL {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            out.push(node.leaf_seg());
        } else {
            self.collect(node.left(), out);
            self.collect(node.right(), out);
        }
    }
}

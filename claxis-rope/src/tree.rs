//! Дерево rope: последовательность текста как диапазонов одного буфера.
//!
//! Дерево сбалансировано по высоте. Спуск к позиции идёт за O(log n) по длинам
//! поддеревьев, которые кэшируются в ветке.

use crate::node::{FREE, Leaf, NIL, Node};

/// Приёмник куска текста при обходе.
pub trait Sink {
    /// Текст диапазона.
    fn piece(&mut self, start: u32, len: u32);
}

/// Пул узлов: выделяется сразу и больше не растёт.
///
/// Свободные узлы хранятся в самих освобождённых узлах: поле `a` держит
/// индекс следующего. Отдельного векла нет, и в горячем пути аллокаций нет.
#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    /// Корень.
    pub root: u32,
    /// Длина документа.
    pub len: u32,
    /// Число листьев: нужна статистика и проверка баланса.
    pub leaves: usize,
    /// Голова списка свободных.
    free_head: u32,
    /// Следующий невыделенный слот.
    next_unused: u32,
}

impl Tree {
    /// Пустое дерево.
    ///
    /// Слот 0 зарезервирован под `NIL`: первый выделенный узел обязан иметь
    /// другой номер, иначе корень неотличим от отсутствия узла.
    pub fn new() -> Self {
        Self {
            nodes: vec![FREE],
            root: NIL,
            len: 0,
            leaves: 0,
            free_head: NIL,
            next_unused: 1,
        }
    }

    /// Дерево с одним листом на весь буфер.
    pub fn with_leaf(start: u32, len: u32) -> Self {
        let mut tree = Self::new();
        if len > 0 {
            tree.root = tree.alloc(Node::leaf(start, len));
            tree.len = len;
            tree.leaves = 1;
        }
        tree
    }

    /// Длина узла: у листа — его длина, у ветки — длина поддерева.
    #[inline]
    pub fn node_len(&self, id: u32) -> u32 {
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            node.leaf_len()
        } else {
            node.len()
        }
    }

    #[inline]
    pub fn node(&self, id: u32) -> Node {
        self.nodes[id as usize]
    }

    /// Занято узлов.
    pub fn node_count(&self) -> usize {
        // Выделено минус один (слот 0 зарезервирован) минус свободные.
        let allocated = self.nodes.len().saturating_sub(1);
        allocated.saturating_sub(self.free_len())
    }

    /// Свободно узлов.
    pub fn free_len(&self) -> usize {
        let mut n = 0;
        let mut cursor = self.free_head;
        while cursor != NIL {
            n += 1;
            cursor = self.nodes[cursor as usize].free_next();
        }
        n
    }

    /// Ёмкость пула.
    pub fn capacity(&self) -> usize {
        self.nodes.capacity()
    }

    /// Выс��а дерева — для проверки баланса.
    pub fn height(&self) -> u32 {
        if self.root == NIL {
            0
        } else {
            self.height_of(self.root)
        }
    }

    /// Высота узла — из самого узла, без обхода.
    fn height_of(&self, id: u32) -> u32 {
        if id == NIL {
            0
        } else {
            self.nodes[id as usize].height()
        }
    }

    /// Новая ветка над двумя поддеревьями: длина и высота считаются сами.
    fn branch(&mut self, l: u32, r: u32) -> u32 {
        let len = self.node_len(l) + self.node_len(r);
        let height = 1 + self.height_of(l).max(self.height_of(r));
        self.alloc(Node::branch(l, r, len, height))
    }

    /// Взять свободный узел или выделить новый.
    #[inline]
    pub fn alloc(&mut self, node: Node) -> u32 {
        if self.free_head != NIL {
            let id = self.free_head;
            self.free_head = self.nodes[id as usize].free_next();
            self.nodes[id as usize] = node;
            return id;
        }
        let id = self.next_unused;
        while self.nodes.len() as u32 <= id {
            self.nodes.push(FREE);
            self.next_unused += 1;
        }
        self.nodes[id as usize] = node;
        id
    }

    /// Вернуть узел в свободные.
    pub fn release(&mut self, id: u32) {
        self.nodes[id as usize] = Node::new(self.free_head & !(1 << 31), 0, 0);
        self.free_head = id;
    }

    /// Вернуть поддерево в свободные.
    pub fn release_tree(&mut self, id: u32) {
        if id == NIL {
            return;
        }
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            self.leaves -= 1;
        } else {
            let (l, r) = (node.left(), node.right());
            self.release_tree(l);
            self.release_tree(r);
        }
        self.release(id);
    }

    /// Пул в исходное состояние за O(1).
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.nodes.push(FREE);
        self.root = NIL;
        self.len = 0;
        self.leaves = 0;
        self.free_head = NIL;
        self.next_unused = 1;
    }

    /// Схлопнуть дерево в один лист, не перевыделяя пул.
    pub fn collapse_to(&mut self, start: u32, len: u32) {
        self.reset();
        if len > 0 {
            self.root = self.alloc(Node::leaf(start, len));
            self.len = len;
            self.leaves = 1;
        }
    }

    // ── сборка ───────────────────────────────────────────────────────────

    /// Склеить два поддерева с балансировкой по высоте.
    pub fn concat(&mut self, left: u32, right: u32) -> u32 {
        if left == NIL {
            return right;
        }
        if right == NIL {
            return left;
        }
        if self.height_of(left) > self.height_of(right) + 1 {
            let node = self.nodes[left as usize];
            let mid = self.concat(node.right(), right);
            let branch = self.branch(node.left(), mid);
            self.release(left);
            branch
        } else if self.height_of(right) > self.height_of(left) + 1 {
            let node = self.nodes[right as usize];
            let mid = self.concat(left, node.left());
            let branch = self.branch(mid, node.right());
            self.release(right);
            branch
        } else {
            self.branch(left, right)
        }
    }

    /// Вставить лист `leaf` на позицию `pos` документа.
    pub fn insert_leaf(&mut self, pos: u32, leaf: u32) {
        self.leaves += 1;
        let len = self.node_len(leaf);
        self.root = self.insert_rec(self.root, 0, pos, leaf);
        self.len += len;
    }

    fn insert_rec(&mut self, idx: u32, base: u32, pos: u32, leaf: u32) -> u32 {
        if idx == NIL {
            return leaf;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let off = pos - base;
            if off == 0 {
                return self.concat(leaf, idx);
            }
            if off == node.leaf_len() {
                return self.concat(idx, leaf);
            }
            // Лист разрезается на отступе: голова и хвост, лист встаёт между.
            let (head, tail) = self.halve(idx, off, off);
            let left = if head == NIL {
                leaf
            } else {
                self.concat(head, leaf)
            };
            let joined = if tail == NIL {
                left
            } else {
                self.concat(left, tail)
            };
            // Здесь тоже нужен поворот: иначе перекос копится вниз, и дерево
            // вырождается, а спуск перестаёт быть логарифмическим.
            self.rebalance(joined)
        } else {
            let (l, r) = (node.left(), node.right());
            let right_base = base + self.node_len(l);
            // Ровно на границе идём вправо: точка вставки — начало правого
            // поддерева. Иначе вставка в конец всегда уходила бы влево и
            // дерево вырождалось бы в список.
            let new = if pos < right_base {
                let l = self.insert_rec(l, base, pos, leaf);
                self.branch(l, r)
            } else {
                let r = self.insert_rec(r, right_base, pos, leaf);
                self.branch(l, r)
            };
            self.release(idx);
            self.rebalance(new)
        }
    }

    /// Заменить диапазон `pos..pos+removed` листом `leaf`.
    ///
    /// Один проход вместо двух: вырезать, потом вставить. Дерево перестраивается
    /// по пути один раз, повороты считаются один раз, а не на каждый проход.
    pub fn replace(&mut self, pos: u32, removed: u32, leaf: u32) {
        // Ничего не вырезается — это обычная вставка, и она уже умеет ходить
        // по краям дерева. Своя дорожка здесь ломалась на вставке в самое
        // начало: диапазон пустой, а узел считался не пересекающимся.
        if removed == 0 {
            self.insert_leaf(pos, leaf);
            return;
        }
        let gained = self.node_len(leaf);
        if self.root == NIL {
            self.root = leaf;
            self.len += gained;
            return;
        }
        self.root = self.replace_rec(self.root, 0, pos, pos + removed, leaf);
        self.len = self.len - removed + gained;
    }

    /// Замена внутри поддерева. `leaf` ставится ровно один раз — в том
    /// поддереве, где начинается вырезаемый диапазон.
    fn replace_rec(&mut self, idx: u32, base: u32, start: u32, end: u32, leaf: u32) -> u32 {
        if idx == NIL {
            return leaf;
        }
        let node_end = base + self.node_len(idx);
        if end <= base || start >= node_end {
            return idx;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let at = node.start();
            let len = node.leaf_len();
            let head_len = start.saturating_sub(base);
            let tail_at = (end - base).min(len);
            let tail_len = len - tail_at;
            let head = if head_len > 0 {
                self.alloc(Node::leaf(at, head_len))
            } else {
                NIL
            };
            let tail = if tail_len > 0 {
                self.alloc(Node::leaf(at + tail_at, tail_len))
            } else {
                NIL
            };
            self.release(idx);
            let left = if head == NIL {
                leaf
            } else {
                self.concat(head, leaf)
            };
            if tail == NIL {
                left
            } else {
                self.concat(left, tail)
            }
        } else {
            let (l, r) = (node.left(), node.right());
            let right_base = base + self.node_len(l);
            // Новый кусок ставится туда, где начинается диапазон. Если он
            // начинается в правом поддереве, левое только вырезается.
            let new = if start < right_base {
                let l = self.replace_rec(l, base, start, end, leaf);
                let r = self.cut_rec(r, right_base, start, end);
                // Через concat, а не через branch: вырезанное поддерево может
                // схлопнуться в пустоту, а ветка с пустым ребёнком — это уже
                // не дерево.
                self.concat(l, r)
            } else {
                let l = self.cut_rec(l, base, start, end);
                let r = self.replace_rec(r, right_base, start, end, leaf);
                self.concat(l, r)
            };
            self.release(idx);
            self.rebalance(new)
        }
    }

    /// Вырезать диапазон `start..end` документа.
    pub fn cut(&mut self, start: u32, end: u32) {
        if start >= end || self.root == NIL {
            return;
        }
        self.root = self.cut_rec(self.root, 0, start, end);
        self.len -= end - start;
    }

    fn cut_rec(&mut self, idx: u32, base: u32, start: u32, end: u32) -> u32 {
        if idx == NIL {
            return NIL;
        }
        let node_end = base + self.node_len(idx);
        if end <= base || start >= node_end {
            return idx;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let from = start.saturating_sub(base);
            let to = (end - base).min(node.leaf_len());
            if from >= to {
                return idx;
            }
            // Из листа остаются голова и хвост, среднее вырезается. Хвост
            // начинается с `to`, а не с `from`: иначе вырезанное осталось бы
            // висеть в хвосте листа.
            let (head, tail) = self.halve(idx, from, to);
            if head == NIL {
                tail
            } else if tail == NIL {
                head
            } else {
                self.concat(head, tail)
            }
        } else {
            let (l, r) = (node.left(), node.right());
            let right_base = base + self.node_len(l);
            let nl = self.cut_rec(l, base, start, end);
            let nr = self.cut_rec(r, right_base, start, end);
            self.release(idx);
            self.concat(nl, nr)
        }
    }

    /// Разрезать лист `id`: оставить `from..len` и `to..len`, среднее выкинуть.
    ///
    /// Один узел превращается в два, поэтому листьев становится на один
    /// больше; исходный слот уходит в свободные.
    fn halve(&mut self, id: u32, from: u32, to: u32) -> (u32, u32) {
        let node = self.nodes[id as usize];
        let at = node.start();
        let len = node.leaf_len();
        let head = if from > 0 {
            self.alloc(Node::leaf(at, from))
        } else {
            NIL
        };
        let tail = if to < len {
            self.alloc(Node::leaf(at + to, len - to))
        } else {
            NIL
        };
        self.release(id);
        self.leaves += 1;
        (head, tail)
    }

    /// Пересобрать узел с балансировкой по высоте.
    /// Выправить узел, если он разъехался по высоте больше чем на единицу.
    ///
    /// Поворотов два вида: простой, когда тяжёлая сторона тяжелее и сама, и
    /// двойной, когда тяжёлый ребёнок тяжелее с другой стороны — тогда сперва
    /// выправляется он, и только потом сам узел.
    fn rebalance(&mut self, idx: u32) -> u32 {
        if idx == NIL {
            return NIL;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            return idx;
        }
        let (l, r) = (node.left(), node.right());
        let (hl, hr) = (self.height_of(l), self.height_of(r));
        if hl > hr + 1 {
            let ln = self.nodes[l as usize];
            if ln.is_leaf() {
                return idx;
            }
            let (ll, lr) = (ln.left(), ln.right());
            // `(ll, lr, r)`: правый поворот даёт `ll` и `(lr, r)`. Если
            // перевесил правый ребёнок `lr`, сперва выправляется он, иначе
            // перекос останется, только переедет на уровень выше.
            if !self.nodes[lr as usize].is_leaf() && self.height_of(lr) > self.height_of(ll) {
                let lrn = self.nodes[lr as usize];
                let new_l = self.branch(ll, lrn.left());
                let new_r = self.branch(lrn.right(), r);
                self.release(lr);
                self.release(l);
                self.release(idx);
                return self.branch(new_l, new_r);
            }
            let new_r = self.branch(lr, r);
            self.release(l);
            self.release(idx);
            return self.branch(ll, new_r);
        }
        if hr > hl + 1 {
            let rn = self.nodes[r as usize];
            if rn.is_leaf() {
                return idx;
            }
            let (rl, rr) = (rn.left(), rn.right());
            if !self.nodes[rl as usize].is_leaf() && self.height_of(rl) > self.height_of(rr) {
                let rln = self.nodes[rl as usize];
                let new_l = self.branch(l, rln.left());
                let new_r = self.branch(rln.right(), rr);
                self.release(rl);
                self.release(r);
                self.release(idx);
                return self.branch(new_l, new_r);
            }
            let new_l = self.branch(l, rl);
            self.release(r);
            self.release(idx);
            return self.branch(new_l, rr);
        }
        idx
    }

    // ── чтение ────────────────────────────────────────────────────────────

    /// Отдать весь документ.
    pub fn read_into<S: Sink>(&self, sink: &mut S) {
        self.walk(self.root, sink);
    }

    fn walk<S: Sink>(&self, idx: u32, sink: &mut S) {
        if idx == NIL {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            sink.piece(node.start(), node.leaf_len());
        } else {
            self.walk(node.left(), sink);
            self.walk(node.right(), sink);
        }
    }

    /// Отдать диапазон документа, не выходя за него.
    pub fn read_range_into<S: Sink>(&self, start: u32, end: u32, sink: &mut S) {
        if start >= end {
            return;
        }
        self.range_rec(self.root, 0, start, end, sink);
    }

    fn range_rec<S: Sink>(&self, idx: u32, base: u32, start: u32, end: u32, sink: &mut S) {
        if idx == NIL {
            return;
        }
        let node_end = base + self.node_len(idx);
        if end <= base || start >= node_end {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            let from = start.saturating_sub(base);
            let to = (end - base).min(node.leaf_len());
            if from < to {
                sink.piece(node.start() + from, to - from);
            }
            return;
        }
        let (l, r) = (node.left(), node.right());
        let right_base = base + self.node_len(l);
        self.range_rec(l, base, start, end, sink);
        self.range_rec(r, right_base, start, end, sink);
    }

    /// Все листья по порядку документа — для проверок и статистики.
    pub fn leaves_in_order(&self) -> Vec<Leaf> {
        let mut out = Vec::with_capacity(self.leaves);
        self.collect(self.root, &mut out);
        out
    }

    fn collect(&self, idx: u32, out: &mut Vec<Leaf>) {
        if idx == NIL {
            return;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            out.push(Leaf {
                start: node.start(),
                len: node.leaf_len(),
            });
        } else {
            self.collect(node.left(), out);
            self.collect(node.right(), out);
        }
    }

    /// Проверка связности дерева: зацикленности быть не должно.
    ///
    /// Зацикливание — это поломка, а не повод падать: диагностика обязана
    /// показать, что именно сломалось. Поэтому дерево, которое себя не
    /// обходит, даёт `false`, а тест на это смотрит.
    pub fn debug_walk(&self) -> (usize, usize, Option<u32>) {
        let mut seen = std::collections::HashSet::new();
        match self.check(self.root, &mut seen, 0) {
            Some(nodes) => (nodes, self.height() as usize, None),
            None => (0, 0, Some(0)),
        }
    }

    /// Дерево обходится целиком и без повторов узлов.
    ///
    /// `None` — дерево зациклено или слишком глубокое.
    pub fn debug_sound(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        self.check(self.root, &mut seen, 0).is_some()
    }

    /// Глубина, дальше которой обход считается зацикленным.
    const MAX_DEPTH: u32 = 128;

    fn check(
        &self,
        idx: u32,
        seen: &mut std::collections::HashSet<u32>,
        depth: u32,
    ) -> Option<usize> {
        if idx == NIL {
            return Some(0);
        }
        if depth >= Self::MAX_DEPTH || !seen.insert(idx) {
            return None;
        }
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            Some(1)
        } else {
            let left = self.check(node.left(), seen, depth + 1)?;
            let right = self.check(node.right(), seen, depth + 1)?;
            Some(left + right + 1)
        }
    }

    #[allow(clippy::type_complexity)]
    fn check2(
        &self,
        idx: u32,
        seen: &mut std::collections::HashSet<u32>,
    ) -> (usize, u32, u32, u32) {
        if idx == NIL {
            return (0, 0, 0, 0);
        }
        seen.insert(idx);
        let node = self.nodes[idx as usize];
        if node.is_leaf() {
            return (1, 1, 0, 0);
        }
        let (ln, lh, _li, lm) = self.check2(node.left(), seen);
        let (rn, rh, ri, rm) = self.check2(node.right(), seen);
        let imb = lh.abs_diff(rh);
        let real = 1 + lh.max(rh);
        let stored = node.height();
        let mismatch = if stored != real { u32::MAX } else { 0 };
        (
            ln + rn + 1,
            real,
            imb.max(ri).max(ri),
            mismatch.max(lm).max(rm),
        )
    }

    /// Диагностика: узлов, реальная высота, худший перекос, расхождение высот.
    pub fn debug_stats(&self) -> (usize, u32, u32, u32) {
        let mut seen = std::collections::HashSet::new();
        self.check2(self.root, &mut seen)
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

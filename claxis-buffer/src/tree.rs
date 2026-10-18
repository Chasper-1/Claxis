//! Дерево rope: последовательность текста как диапазонов одного буфера.
//!
//! Дерево сбалансировано по высоте. Спуск к позиции идёт за O(log n) по длинам
//! поддеревьев, которые кэшируются в ветке.

use crate::node::{FREE, Leaf, MAX_NODES, NIL, Node};

/// Глубина, дальше которой обход считается зацикленным.
const MAX_DEPTH: u32 = 128;

/// Приёмник куска текста при обходе.
pub trait Sink {
    /// Диапазон буфера: откуда и сколько.
    fn range(&mut self, start: u32, len: u32);
}

/// Сводка проверки дерева.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TreeStats {
    /// Сколько узлов в дереве.
    pub nodes: usize,
    /// Реальная высота, посчитанная обходом.
    pub height: u32,
    /// Худший перекос между высотами детей в любом узле.
    pub worst_imbalance: u32,
    /// Ноль, если все высоты в узлах сходятся с настоящими; иначе — метка
    /// расхождения.
    pub height_mismatch: u32,
}

impl TreeStats {
    /// Пустая сводка: узла нет.
    const EMPTY: TreeStats = TreeStats {
        nodes: 0,
        height: 0,
        worst_imbalance: 0,
        height_mismatch: 0,
    };
}

/// Пул узлов: выделяется сразу и больше не растёт.
///
/// Свободные узлы хранятся в самих освобождённых узлах: поле `a` держит
/// индекс следующего. Отдельного вектора нет, и в горячем пути аллокаций нет.
///
/// Вид узла — лист или ветка и её высота — упакован в сам узел: спуск читает
/// один массив `nodes`, а не второй рядом.
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
    /// Сколько узлов в списке свободных.
    ///
    /// Считается на месте: без него `free_len` обходил бы весь список, а его
    /// спрашивают и статистика памяти, и число занятых узлов.
    free_count: usize,
    /// Самое позднее место буфера среди когда-либо выделенных листьев.
    ///
    /// Обновляется при выделении листа и не уменьшается до [`Tree::reset`]:
    /// это верхняя граница, а не точный максимум по живым листьям. Так её и
    /// спрашивает обрезка буфера — обрезать меньше безопасно, а обходить всё
    /// дерево на каждой правке ради байта-двух нет.
    leaf_watermark: u32,
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
            free_count: 0,
            leaf_watermark: 0,
        }
    }

    /// Дерево с одним листом на весь буфер.
    pub fn with_leaf(start: u32, len: u32) -> Self {
        let mut tree = Self::new();
        if len > 0 {
            tree.root = tree.alloc_leaf(start, len);
            tree.len = len;
        }
        tree
    }

    /// Лист ли узел.
    #[inline]
    fn is_leaf(&self, id: u32) -> bool {
        self.nodes[id as usize].is_leaf()
    }

    /// Высота узла: у листа — единица, у ветки — из байта вида.
    #[inline]
    fn node_height(&self, id: u32) -> u32 {
        if id == NIL {
            0
        } else {
            self.nodes[id as usize].height()
        }
    }

    /// Длина текста узла: у листа — его длина, у ветки — длина поддерева.
    #[inline]
    pub fn node_text_len(&self, id: u32) -> u32 {
        // Обращение к пулу одно: вид узла лежит в нём самом, а не рядом.
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            node.leaf_len()
        } else {
            node.subtree_len()
        }
    }

    /// Узел по номеру.
    #[inline]
    pub fn node_at(&self, id: u32) -> Node {
        self.nodes[id as usize]
    }

    /// Занято узлов.
    pub fn node_count(&self) -> usize {
        self.node_count_with(self.free_len())
    }

    /// Занято узлов, если число свободных уже известно.
    ///
    /// `free_len` обходит свободный список, а вызывающему он может быть нужен
    /// сам. Тогда обход делается один раз, а не два.
    pub fn node_count_with(&self, free: usize) -> usize {
        // Выделено минус один (слот 0 зарезервирован) минус свободные.
        let allocated = self.nodes.len().saturating_sub(1);
        allocated.saturating_sub(free)
    }

    /// Свободно узлов.
    #[inline]
    pub fn free_len(&self) -> usize {
        self.free_count
    }

    /// Ёмкость пула.
    pub fn capacity(&self) -> usize {
        self.nodes.capacity()
    }

    /// Занято слотов пула, вместе с зарезервированным под `NIL`.
    pub fn slot_len(&self) -> usize {
        self.nodes.len()
    }

    /// Помещается ли в индекс ещё `extra` слотов.
    ///
    /// Индекс узла — двадцать четыре бита, и упереться в него пул может молча:
    /// новый узел получил бы обрезанный номер и указывал бы не туда. Поэтому
    /// запас спрашивают заранее — до правки, а не после.
    pub fn has_room_for_nodes(&self, extra: usize) -> bool {
        self.nodes.len().saturating_add(extra) <= MAX_NODES
    }

    /// Высота дерева — для проверки баланса.
    pub fn height(&self) -> u32 {
        if self.root == NIL {
            0
        } else {
            self.node_height(self.root)
        }
    }

    /// Новая ветка над двумя поддеревьями: длина и высота считаются сами.
    fn make_branch(&mut self, left: u32, right: u32) -> u32 {
        // Оба ребёнка читаются из пула один раз: и длина, и высота лежат в
        // самом узле, а лишние обращения — это лишние проверки границ в самом
        // горячем пути.
        let left_node = self.nodes[left as usize];
        let right_node = self.nodes[right as usize];
        let (left_len, left_height) = if left_node.is_leaf() {
            (left_node.leaf_len(), 1)
        } else {
            (left_node.subtree_len(), left_node.height())
        };
        let (right_len, right_height) = if right_node.is_leaf() {
            (right_node.leaf_len(), 1)
        } else {
            (right_node.subtree_len(), right_node.height())
        };
        let len = left_len + right_len;
        let height = 1 + left_height.max(right_height);
        self.alloc_node(Node::branch(left, right, len, height))
    }

    /// Выделить лист.
    #[inline]
    pub fn alloc_leaf(&mut self, start: u32, len: u32) -> u32 {
        let id = self.alloc_node(Node::leaf(start, len));
        self.leaves += 1;
        let end = start.saturating_add(len);
        if end > self.leaf_watermark {
            self.leaf_watermark = end;
        }
        id
    }

    /// Взять свободный узел или выделить новый. Вид узла выставляет вызывающий.
    ///
    /// Счётчик листьев живёт здесь и в [`Tree::release`] и больше нигде: узел
    /// появляется и исчезает только через эти две функции, а счётчик,
    /// который надо помнить в каждом месте отдельно, разъезжается — уже
    /// разъезжался.
    #[inline]
    fn alloc_node(&mut self, node: Node) -> u32 {
        let id = if self.free_head != NIL {
            let id = self.free_head;
            self.free_head = self.nodes[id as usize].free_next();
            self.free_count -= 1;
            id
        } else {
            let id = self.nodes.len() as u32;
            self.nodes.push(FREE);
            id
        };
        self.nodes[id as usize] = node;
        id
    }

    /// Вернуть узел в свободные.
    #[inline]
    pub fn release(&mut self, id: u32) {
        // Узел читается один раз: и вид его, и запись в список берутся из
        // одного обращения к пулу, а не из двух.
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            self.leaves -= 1;
        }
        self.nodes[id as usize] = Node::free(self.free_head);
        self.free_head = id;
        self.free_count += 1;
    }

    /// Вернуть поддерево в свободные.
    #[cfg(test)]
    pub fn release_tree(&mut self, id: u32) {
        if id == NIL {
            return;
        }
        let node = self.nodes[id as usize];
        if !node.is_leaf() {
            let (left, right) = (node.left(), node.right());
            self.release_tree(left);
            self.release_tree(right);
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
        self.free_count = 0;
        self.leaf_watermark = 0;
    }

    /// Схлопнуть дерево в один лист, не перевыделяя пул.
    pub fn collapse_to(&mut self, start: u32, len: u32) {
        self.reset();
        if len > 0 {
            self.root = self.alloc_leaf(start, len);
            self.len = len;
        }
    }

    // ── сборка ───────────────────────────────────────────────────────────

    /// Склеить два поддерева с балансировкой по высоте.
    ///
    /// Соседние листья со смежными диапазонами буфера склеиваются в один: в
    /// дереве они описывают один кусок текста подряд, а значит и диапазоны
    /// буфера идут вплотную. Лист — это диапазон буфера, а не ссылка на
    /// запись, поэтому склейка ничего не ломает: ни текст, ни история не
    /// хранят границы листьев.
    ///
    /// Склеиваются не все соседи, а только те, чьи куски в буфере идут
    /// вплотную. Разрыв означает, что между ними лежит текст, который
    /// документ уже не показывает, но отмена его вернёт, — такой лист
    /// разорван не зря, и место между диапазонами закрывать нельзя.
    pub fn concat(&mut self, left: u32, right: u32) -> u32 {
        if left == NIL {
            return right;
        }
        if right == NIL {
            return left;
        }
        if self.adjacent_leaves(left, right) {
            return self.merge_adjacent_leaves(left, right);
        }
        if self.node_height(left) > self.node_height(right) + 1 {
            let left_node = self.nodes[left as usize];
            let middle = self.concat(left_node.right(), right);
            let joined = self.make_branch(left_node.left(), middle);
            self.release(left);
            // Свежий узел после спуска по правому ребёнку может разъехаться
            // на два: классический join обязан повернуть на каждом уровне, а
            // не только на верхнем. Без этого перекос копится в глубине.
            self.rebalance(joined)
        } else if self.node_height(right) > self.node_height(left) + 1 {
            let right_node = self.nodes[right as usize];
            let middle = self.concat(left, right_node.left());
            let joined = self.make_branch(middle, right_node.right());
            self.release(right);
            self.rebalance(joined)
        } else {
            self.make_branch(left, right)
        }
    }

    /// Идут ли два листа в буфере вплотную друг к другу.
    fn adjacent_leaves(&self, left: u32, right: u32) -> bool {
        let left_node = self.nodes[left as usize];
        let right_node = self.nodes[right as usize];
        // Вид берётся из уже прочитанных узлов: отдельные `is_leaf` читали бы
        // пул ещё раз.
        left_node.is_leaf()
            && right_node.is_leaf()
            && right_node.start() == left_node.start().saturating_add(left_node.leaf_len())
    }

    /// Склеить два смежных листа в один.
    ///
    /// Левый узел переписывается на месте: он уже принадлежит дереву, а
    /// новый узел только ради удлинения диапазона взял бы слот из пула.
    fn merge_adjacent_leaves(&mut self, left: u32, right: u32) -> u32 {
        let left_node = self.nodes[left as usize];
        let len = left_node.leaf_len() + self.nodes[right as usize].leaf_len();
        self.nodes[left as usize] = Node::leaf(left_node.start(), len);
        self.release(right);
        left
    }

    /// Вставить лист `leaf` на позицию `pos` документа.
    ///
    /// Пустой лист — не лист: он бы не нёс текста, но занял бы место в дереве
    /// и разошёлся бы со счётчиком листьев. Такой просто возвращается в пул.
    pub fn insert_leaf(&mut self, pos: u32, leaf: u32) {
        let len = self.node_text_len(leaf);
        if len == 0 {
            self.release(leaf);
            return;
        }
        self.root = self.insert_at(self.root, 0, pos, leaf);
        self.len += len;
    }

    fn insert_at(&mut self, id: u32, subtree_start: u32, pos: u32, leaf: u32) -> u32 {
        if id == NIL {
            return leaf;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            let offset = pos - subtree_start;
            if offset == 0 {
                return self.concat(leaf, id);
            }
            if offset == node.leaf_len() {
                return self.concat(id, leaf);
            }
            // Лист разрезается на отступе: голова и хвост, лист встаёт между.
            let (head, tail) = self.split_leaf_around_hole(id, offset, offset);
            let left = if head == NIL {
                leaf
            } else {
                self.concat(head, leaf)
            };
            // Поворот не нужен: `concat` сам держит высоты в узле на расстоянии
            // не больше единицы.
            if tail == NIL {
                left
            } else {
                self.concat(left, tail)
            }
        } else {
            let (left, right) = (node.left(), node.right());
            let right_start = subtree_start + self.node_text_len(left);
            // Ровно на границе идём вправо: точка вставки — начало правого
            // поддерева. Иначе вставка в конец всегда уходила бы влево и
            // дерево вырождалось бы в список.
            let updated = if pos < right_start {
                let left = self.insert_at(left, subtree_start, pos, leaf);
                self.make_branch(left, right)
            } else {
                let right = self.insert_at(right, right_start, pos, leaf);
                self.make_branch(left, right)
            };
            self.release(id);
            self.rebalance(updated)
        }
    }

    /// Заменить диапазон `pos..pos+removed` листом `leaf`.
    ///
    /// Один проход вместо двух: вырезать, потом вставить. Дерево перестраивается
    /// по пути один раз, повороты считаются один раз, а не на каждый проход.
    pub fn replace(&mut self, pos: u32, removed: u32, leaf: u32) {
        // Пустой лист ничего не вставляет: только вырезается диапазон.
        if self.node_text_len(leaf) == 0 {
            self.release(leaf);
            if removed > 0 {
                self.cut(pos, pos + removed);
            }
            return;
        }
        // Ничего не вырезается — это обычная вставка, и она уже умеет ходить
        // по краям дерева. Своя дорожка здесь ломалась на вставке в самое
        // начало: диапазон пустой, а узел считался не пересекающимся.
        if removed == 0 {
            self.insert_leaf(pos, leaf);
            return;
        }
        let gained = self.node_text_len(leaf);
        if self.root == NIL {
            self.root = leaf;
            self.len += gained;
            return;
        }
        self.root = self.replace_within(self.root, 0, pos, pos + removed, leaf);
        self.len = self.len - removed + gained;
    }

    /// Замена внутри поддерева. `leaf` ставится ровно один раз — в том
    /// поддереве, где начинается вырезаемый диапазон.
    fn replace_within(
        &mut self,
        id: u32,
        subtree_start: u32,
        start: u32,
        end: u32,
        leaf: u32,
    ) -> u32 {
        if id == NIL {
            return leaf;
        }
        let subtree_end = subtree_start + self.node_text_len(id);
        if end <= subtree_start || start >= subtree_end {
            return id;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            let buffer_start = node.start();
            let len = node.leaf_len();
            let head_len = start.saturating_sub(subtree_start);
            let tail_start = (end - subtree_start).min(len);
            let tail_len = len - tail_start;
            let head = if head_len > 0 {
                self.alloc_leaf(buffer_start, head_len)
            } else {
                NIL
            };
            let tail = if tail_len > 0 {
                self.alloc_leaf(buffer_start + tail_start, tail_len)
            } else {
                NIL
            };
            self.release(id);
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
            let (left, right) = (node.left(), node.right());
            let right_start = subtree_start + self.node_text_len(left);
            // Новый кусок ставится туда, где начинается диапазон. Если он
            // начинается в правом поддереве, левое только вырезается.
            let updated = if start < right_start {
                let left = self.replace_within(left, subtree_start, start, end, leaf);
                let right = self.cut_within(right, right_start, start, end);
                // Через concat, а не через make_branch: вырезанное поддерево
                // может схлопнуться в пустоту, а ветка с пустым ребёнком — это
                // уже не дерево.
                self.concat(left, right)
            } else {
                let left = self.cut_within(left, subtree_start, start, end);
                let right = self.replace_within(right, right_start, start, end, leaf);
                self.concat(left, right)
            };
            self.release(id);
            // Поворот не нужен: `concat` сам держит высоты в узле на расстоянии
            // не больше единицы.
            updated
        }
    }

    /// Вырезать диапазон `start..end` документа.
    pub fn cut(&mut self, start: u32, end: u32) {
        if start >= end || self.root == NIL {
            return;
        }
        self.root = self.cut_within(self.root, 0, start, end);
        self.len -= end - start;
    }

    fn cut_within(&mut self, id: u32, subtree_start: u32, start: u32, end: u32) -> u32 {
        if id == NIL {
            return NIL;
        }
        let subtree_end = subtree_start + self.node_text_len(id);
        if end <= subtree_start || start >= subtree_end {
            return id;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            let hole_start = start.saturating_sub(subtree_start);
            let hole_end = (end - subtree_start).min(node.leaf_len());
            if hole_start >= hole_end {
                return id;
            }
            // Из листа остаются голова и хвост, среднее вырезается. Хвост
            // начинается с `hole_end`, а не с `hole_start`: иначе вырезанное
            // осталось бы висеть в хвосте листа.
            let (head, tail) = self.split_leaf_around_hole(id, hole_start, hole_end);
            if head == NIL {
                tail
            } else if tail == NIL {
                head
            } else {
                self.concat(head, tail)
            }
        } else {
            let (left, right) = (node.left(), node.right());
            let right_start = subtree_start + self.node_text_len(left);
            let new_left = self.cut_within(left, subtree_start, start, end);
            let new_right = self.cut_within(right, right_start, start, end);
            self.release(id);
            self.concat(new_left, new_right)
        }
    }

    /// Разрезать лист `id` на голову `0..hole_start` и хвост `hole_end..len`,
    /// выкинув кусок между ними.
    ///
    /// Один узел превращается в два, поэтому листьев становится на один больше;
    /// исходный слот уходит в свободные. Счётчик листьев поправляют
    /// [`Tree::alloc_leaf`] и [`Tree::release`], здесь он не трогается.
    fn split_leaf_around_hole(&mut self, id: u32, hole_start: u32, hole_end: u32) -> (u32, u32) {
        let node = self.nodes[id as usize];
        let buffer_start = node.start();
        let len = node.leaf_len();
        let head = if hole_start > 0 {
            self.alloc_leaf(buffer_start, hole_start)
        } else {
            NIL
        };
        let tail = if hole_end < len {
            self.alloc_leaf(buffer_start + hole_end, len - hole_end)
        } else {
            NIL
        };
        self.release(id);
        (head, tail)
    }

    /// Выправить узел, если он разъехался по высоте больше чем на единицу.
    ///
    /// Поворотов два вида: простой, когда тяжёлая сторона тяжелее и сама, и
    /// двойной, когда тяжёлый ребёнок тяжелее с другой стороны — тогда сперва
    /// выправляется он, и только потом сам узел.
    fn rebalance(&mut self, id: u32) -> u32 {
        if id == NIL {
            return NIL;
        }
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            return id;
        }
        let (left, right) = (node.left(), node.right());
        let (left_height, right_height) = (self.node_height(left), self.node_height(right));
        if left_height > right_height + 1 {
            let left_node = self.nodes[left as usize];
            if left_node.is_leaf() {
                return id;
            }
            let (left_left, left_right) = (left_node.left(), left_node.right());
            // `(left_left, left_right, right)`: правый поворот даёт `left_left`
            // и `(left_right, right)`. Если перевесил правый ребёнок
            // `left_right`, сперва выправляется он, иначе перекос останется,
            // только переедет на уровень выше.
            if !self.is_leaf(left_right)
                && self.node_height(left_right) > self.node_height(left_left)
            {
                let left_right_node = self.nodes[left_right as usize];
                let new_left = self.make_branch(left_left, left_right_node.left());
                let new_right = self.make_branch(left_right_node.right(), right);
                self.release(left_right);
                self.release(left);
                self.release(id);
                return self.make_branch(new_left, new_right);
            }
            let new_right = self.make_branch(left_right, right);
            self.release(left);
            self.release(id);
            return self.make_branch(left_left, new_right);
        }
        if right_height > left_height + 1 {
            let right_node = self.nodes[right as usize];
            if right_node.is_leaf() {
                return id;
            }
            let (right_left, right_right) = (right_node.left(), right_node.right());
            if !self.is_leaf(right_left)
                && self.node_height(right_left) > self.node_height(right_right)
            {
                let right_left_node = self.nodes[right_left as usize];
                let new_left = self.make_branch(left, right_left_node.left());
                let new_right = self.make_branch(right_left_node.right(), right_right);
                self.release(right_left);
                self.release(right);
                self.release(id);
                return self.make_branch(new_left, new_right);
            }
            let new_left = self.make_branch(left, right_left);
            self.release(right);
            self.release(id);
            return self.make_branch(new_left, right_right);
        }
        id
    }

    // ── чтение ────────────────────────────────────────────────────────────

    /// Отдать весь документ.
    pub fn read_into<S: Sink>(&self, sink: &mut S) {
        self.read_subtree_into(self.root, sink);
    }

    fn read_subtree_into<S: Sink>(&self, id: u32, sink: &mut S) {
        if id == NIL {
            return;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            sink.range(node.start(), node.leaf_len());
        } else {
            self.read_subtree_into(node.left(), sink);
            self.read_subtree_into(node.right(), sink);
        }
    }

    /// Отдать диапазон документа, не выходя за него.
    pub fn read_range_into<S: Sink>(&self, start: u32, end: u32, sink: &mut S) {
        if start >= end {
            return;
        }
        self.read_range_within(self.root, 0, start, end, sink);
    }

    fn read_range_within<S: Sink>(
        &self,
        id: u32,
        subtree_start: u32,
        start: u32,
        end: u32,
        sink: &mut S,
    ) {
        if id == NIL {
            return;
        }
        let subtree_end = subtree_start + self.node_text_len(id);
        if end <= subtree_start || start >= subtree_end {
            return;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            let from = start.saturating_sub(subtree_start);
            let to = (end - subtree_start).min(node.leaf_len());
            if from < to {
                sink.range(node.start() + from, to - from);
            }
            return;
        }
        let (left, right) = (node.left(), node.right());
        let right_start = subtree_start + self.node_text_len(left);
        self.read_range_within(left, subtree_start, start, end, sink);
        self.read_range_within(right, right_start, start, end, sink);
    }

    /// Все листья по порядку документа — для проверок и статистики.
    pub fn leaves_in_order(&self) -> Vec<Leaf> {
        let mut leaves = Vec::with_capacity(self.leaves);
        self.collect_leaves(self.root, &mut leaves);
        leaves
    }

    /// Граница буфера, до которой стоит его хранить: за ней ни один лист уже
    /// не читает.
    ///
    /// Верхняя граница, а не точный максимум: она не меньше конца любого
    /// живого листа, но может помнить и раньше освобождённый дальний лист.
    /// Обрезать по ней безопасно, а стоит она O(1) — обход дерева на каждой
    /// правке не нужен.
    #[inline]
    pub fn leaf_end(&self) -> u32 {
        self.leaf_watermark
    }

    fn collect_leaves(&self, id: u32, leaves: &mut Vec<Leaf>) {
        if id == NIL {
            return;
        }
        let node = self.nodes[id as usize];
        if self.is_leaf(id) {
            leaves.push(Leaf {
                start: node.start(),
                len: node.leaf_len(),
            });
        } else {
            self.collect_leaves(node.left(), leaves);
            self.collect_leaves(node.right(), leaves);
        }
    }

    // ── диагностика ───────────────────────────────────────────────────────

    /// Сколько узлов достижимо обходом, или `None`, если обход зациклился.
    pub fn debug_reachable_nodes(&self) -> Option<usize> {
        // Отметки узлов — по слоту пула: вектор булей дешевле и меньше
        // множества, а номер узла и есть индекс слота.
        let mut seen = vec![false; self.nodes.len()];
        self.count_nodes(self.root, &mut seen, 0)
    }

    /// Дерево обходится целиком и без повторов узлов.
    ///
    /// `false` — дерево зациклено или слишком глубокое.
    pub fn debug_sound(&self) -> bool {
        self.debug_reachable_nodes().is_some()
    }

    fn count_nodes(&self, id: u32, seen: &mut [bool], depth: u32) -> Option<usize> {
        if id == NIL {
            return Some(0);
        }
        // `replace` сразу ставит отметку и говорит, стояла ли она уже.
        if depth >= MAX_DEPTH || std::mem::replace(&mut seen[id as usize], true) {
            return None;
        }
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            Some(1)
        } else {
            let left = self.count_nodes(node.left(), seen, depth + 1)?;
            let right = self.count_nodes(node.right(), seen, depth + 1)?;
            Some(left + right + 1)
        }
    }

    /// Диагностика: узлов, реальная высота, худший перекос, расхождение высот.
    ///
    /// Обход не зависает на зацикленном дереве — на повторном узле или слишком
    /// большой глубине он обрывается. И раз обрыв — это не «пусто», а «дерево
    /// не такое, каким должно быть», сводки в этом случае нет вовсе: `None`
    /// честнее пустой сводки, по которой вышло бы, что всё в порядке.
    pub fn debug_stats(&self) -> Option<TreeStats> {
        let mut seen = vec![false; self.nodes.len()];
        self.collect_stats(self.root, &mut seen, 0)
    }

    fn collect_stats(&self, id: u32, seen: &mut [bool], depth: u32) -> Option<TreeStats> {
        if id == NIL {
            return Some(TreeStats::EMPTY);
        }
        if depth >= MAX_DEPTH || std::mem::replace(&mut seen[id as usize], true) {
            return None;
        }
        let node = self.nodes[id as usize];
        if node.is_leaf() {
            return Some(TreeStats {
                nodes: 1,
                height: 1,
                worst_imbalance: 0,
                height_mismatch: 0,
            });
        }
        let left = self.collect_stats(node.left(), seen, depth + 1)?;
        let right = self.collect_stats(node.right(), seen, depth + 1)?;
        let imbalance = left.height.abs_diff(right.height);
        let real_height = 1 + left.height.max(right.height);
        let mismatch = if self.node_height(id) != real_height {
            u32::MAX
        } else {
            0
        };
        Some(TreeStats {
            nodes: left.nodes + right.nodes + 1,
            height: real_height,
            worst_imbalance: imbalance
                .max(left.worst_imbalance)
                .max(right.worst_imbalance),
            height_mismatch: mismatch
                .max(left.height_mismatch)
                .max(right.height_mismatch),
        })
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Приёмник кусков: складывает их в буфер и запоминает диапазоны.
    struct RangeReader<'a> {
        buffer: &'a [u8],
        text: Vec<u8>,
        ranges: Vec<(u32, u32)>,
    }

    impl<'a> RangeReader<'a> {
        fn new(buffer: &'a [u8]) -> Self {
            Self {
                buffer,
                text: Vec::new(),
                ranges: Vec::new(),
            }
        }
    }

    impl Sink for RangeReader<'_> {
        fn range(&mut self, start: u32, len: u32) {
            self.ranges.push((start, len));
            let from = start as usize;
            self.text
                .extend_from_slice(&self.buffer[from..from + len as usize]);
        }
    }

    /// Буфер, где байт с номером `i` — это `b'a' + i % 26`.
    fn sample_buffer(len: usize) -> Vec<u8> {
        (0..len).map(|i| b'a' + (i % 26) as u8).collect()
    }

    fn pieced_tree(pieces: &[(u32, u32)]) -> Tree {
        let mut tree = Tree::new();
        for &(start, len) in pieces {
            let leaf = tree.alloc_leaf(start, len);
            tree.insert_leaf(tree.len, leaf);
        }
        tree
    }

    #[test]
    fn new_tree_is_empty() {
        let tree = Tree::new();
        assert_eq!(tree.root, NIL);
        assert_eq!(tree.len, 0);
        assert_eq!(tree.leaves, 0);
        assert_eq!(tree.height(), 0);
        assert_eq!(tree.node_count(), 0);
        assert_eq!(tree.free_len(), 0);
        assert!(tree.capacity() >= 1);
        assert!(tree.debug_sound());
        assert_eq!(tree.debug_reachable_nodes(), Some(0));
    }

    #[test]
    fn default_tree_is_empty() {
        let tree = Tree::default();
        assert_eq!(tree.root, NIL);
        assert_eq!(tree.len, 0);
    }

    #[test]
    fn with_leaf_of_zero_keeps_the_tree_empty() {
        let tree = Tree::with_leaf(0, 0);
        assert_eq!(tree.root, NIL);
        assert_eq!(tree.len, 0);
        assert_eq!(tree.leaves, 0);
    }

    #[test]
    fn with_leaf_holds_the_whole_buffer() {
        let tree = Tree::with_leaf(3, 5);
        assert_eq!(tree.len, 5);
        assert_eq!(tree.leaves, 1);
        assert_eq!(tree.height(), 1);
        assert_eq!(tree.node_text_len(tree.root), 5);
        assert_eq!(tree.node_at(tree.root).start(), 3);
        assert_eq!(tree.node_at(tree.root).leaf_len(), 5);
    }

    #[test]
    fn released_leaf_slot_is_reused() {
        let mut tree = Tree::new();
        let first = tree.alloc_leaf(0, 4);
        assert_eq!(tree.leaves, 1);
        assert_eq!(tree.node_count(), 1);
        tree.release(first);
        assert_eq!(tree.leaves, 0);
        assert_eq!(tree.node_count(), 0);
        assert_eq!(tree.free_len(), 1);
        let second = tree.alloc_leaf(0, 4);
        assert_eq!(
            second, first,
            "слот свободного узла должен переиспользоваться"
        );
    }

    #[test]
    fn release_tree_returns_every_node() {
        let mut tree = pieced_tree(&[(0, 3), (5, 3), (10, 3)]);
        assert!(tree.node_count() > 1);
        tree.release_tree(tree.root);
        assert_eq!(tree.node_count(), 0);
        assert_eq!(tree.leaves, 0);
    }

    #[test]
    fn free_count_matches_the_free_list() {
        // Счётчик свободных держится на месте ради O(1) `free_len`: он обязан
        // совпадать с настоящим списком в любой момент.
        fn walked(tree: &Tree) -> usize {
            let mut count = 0;
            let mut cursor = tree.free_head;
            while cursor != NIL {
                count += 1;
                cursor = tree.nodes[cursor as usize].free_next();
            }
            count
        }

        let mut tree = Tree::new();
        let a = tree.alloc_leaf(0, 2);
        let b = tree.alloc_leaf(2, 2);
        assert_eq!(tree.free_len(), 0);
        assert_eq!(tree.free_len(), walked(&tree));

        tree.release(a);
        tree.release(b);
        assert_eq!(tree.free_len(), 2);
        assert_eq!(tree.free_len(), walked(&tree), "после освобождения");

        let _c = tree.alloc_leaf(4, 2);
        assert_eq!(tree.free_len(), 1);
        assert_eq!(tree.free_len(), walked(&tree), "после переиспользования");

        let _d = tree.alloc_leaf(6, 2);
        assert_eq!(tree.free_len(), 0);
        assert_eq!(tree.free_len(), walked(&tree));
        assert_eq!(tree.node_count(), 2);
        assert_eq!(tree.node_count_with(tree.free_len()), tree.node_count());
    }

    #[test]
    fn debug_stats_agrees_with_the_reachable_count() {
        // Отметки обхода живут в векторе по слотам пула: счёт занятых узлов и
        // число достижимых обязаны сойтись.
        let mut tree = Tree::new();
        for i in 0..64u32 {
            let leaf = tree.alloc_leaf(i * 3, 2);
            tree.insert_leaf(tree.len, leaf);
        }
        let stats = tree.debug_stats().expect("дерево не зациклено");
        assert_eq!(Some(stats.nodes), tree.debug_reachable_nodes());
        assert_eq!(stats.nodes, tree.leaves * 2 - 1);
    }

    #[test]
    fn reset_clears_the_pool() {
        let mut tree = pieced_tree(&[(0, 3), (3, 3)]);
        tree.reset();
        assert_eq!(tree.root, NIL);
        assert_eq!(tree.len, 0);
        assert_eq!(tree.leaves, 0);
        assert_eq!(tree.node_count(), 0);
        assert_eq!(tree.free_len(), 0);
    }

    #[test]
    fn the_node_index_has_a_ceiling() {
        // Индекс узла 24-битный: слотов помещается ровно `MAX_NODES`. Слот 0
        // занят под `NIL`, поэтому пул из одного только пустого дерева уже
        // съел одно место.
        let tree = Tree::new();
        assert_eq!(tree.slot_len(), 1);
        assert!(tree.has_room_for_nodes(MAX_NODES - 1), "ровно до потолка");
        assert!(!tree.has_room_for_nodes(MAX_NODES), "на узел больше — нет");
        assert!(!Tree::new().has_room_for_nodes(usize::MAX));
    }

    #[test]
    fn collapse_to_a_single_leaf() {
        let mut tree = pieced_tree(&[(0, 3), (3, 3), (6, 3)]);
        tree.collapse_to(2, 7);
        assert_eq!(tree.len, 7);
        assert_eq!(tree.leaves, 1);
        assert_eq!(tree.height(), 1);
        tree.collapse_to(0, 0);
        assert_eq!(tree.root, NIL);
        assert_eq!(tree.len, 0);
    }

    #[test]
    fn concat_merges_buffer_adjacent_leaves() {
        let mut tree = Tree::new();
        let left = tree.alloc_leaf(0, 3);
        let right = tree.alloc_leaf(3, 4);
        let merged = tree.concat(left, right);
        assert_eq!(tree.leaves, 1, "смежные листья склеиваются в один");
        assert_eq!(tree.node_at(merged).start(), 0);
        assert_eq!(tree.node_at(merged).leaf_len(), 7);
    }

    #[test]
    fn concat_keeps_a_gap_between_leaves() {
        // Между кусками буфера лежит текст, который отмена вернёт: склеивать
        // их нельзя.
        let tree = pieced_tree(&[(0, 3), (5, 4)]);
        assert_eq!(tree.leaves, 2);
        let pieces: Vec<_> = tree
            .leaves_in_order()
            .iter()
            .map(|leaf| (leaf.start, leaf.len))
            .collect();
        assert_eq!(pieces, vec![(0, 3), (5, 4)]);
    }

    #[test]
    fn concat_ignores_nil_children() {
        let mut tree = pieced_tree(&[(0, 3)]);
        let leaf = tree.root;
        assert_eq!(tree.concat(NIL, leaf), leaf);
        assert_eq!(tree.concat(leaf, NIL), leaf);
    }

    #[test]
    fn insert_leaf_into_an_empty_tree() {
        let mut tree = Tree::new();
        let leaf = tree.alloc_leaf(0, 4);
        tree.insert_leaf(0, leaf);
        assert_eq!(tree.len, 4);
        assert_eq!(tree.leaves, 1);
    }

    #[test]
    fn insert_leaf_at_start_and_end() {
        let mut tree = Tree::new();
        let tail = tree.alloc_leaf(4, 3);
        tree.insert_leaf(0, tail);
        let head = tree.alloc_leaf(0, 4);
        tree.insert_leaf(0, head);
        let last = tree.alloc_leaf(7, 2);
        tree.insert_leaf(tree.len, last);
        assert_eq!(tree.len, 9);
        assert_eq!(tree.leaf_end(), 9);
    }

    #[test]
    fn insert_leaf_in_the_middle_splits_the_leaf() {
        let mut tree = Tree::with_leaf(0, 6);
        let leaf = tree.alloc_leaf(100, 2);
        tree.insert_leaf(3, leaf);
        assert_eq!(tree.len, 8);
        assert_eq!(tree.leaves, 3);
        let pieces = tree.leaves_in_order();
        assert_eq!(
            pieces.iter().map(|l| (l.start, l.len)).collect::<Vec<_>>(),
            vec![(0, 3), (100, 2), (3, 3)]
        );
    }

    #[test]
    fn inserting_an_empty_leaf_changes_nothing() {
        let mut tree = Tree::with_leaf(0, 4);
        let empty = tree.alloc_leaf(0, 0);
        tree.insert_leaf(2, empty);
        assert_eq!(tree.len, 4);
        assert_eq!(tree.leaves, 1);
    }

    #[test]
    fn replace_swaps_a_range_for_a_leaf() {
        let mut tree = Tree::with_leaf(0, 10);
        let leaf = tree.alloc_leaf(100, 3);
        tree.replace(2, 4, leaf);
        assert_eq!(tree.len, 9);
        assert_eq!(
            tree.leaves_in_order()
                .iter()
                .map(|l| (l.start, l.len))
                .collect::<Vec<_>>(),
            vec![(0, 2), (100, 3), (6, 4)]
        );
    }

    #[test]
    fn replace_without_removal_is_an_insert() {
        let mut tree = Tree::with_leaf(0, 4);
        let leaf = tree.alloc_leaf(50, 2);
        tree.replace(4, 0, leaf);
        assert_eq!(tree.len, 6);
        assert_eq!(tree.leaf_end(), 50 + 2);
    }

    #[test]
    fn replace_with_an_empty_leaf_is_a_cut() {
        let mut tree = Tree::with_leaf(0, 10);
        let empty = tree.alloc_leaf(0, 0);
        tree.replace(3, 5, empty);
        assert_eq!(tree.len, 5);
        assert_eq!(
            tree.leaves_in_order()
                .iter()
                .map(|l| (l.start, l.len))
                .collect::<Vec<_>>(),
            vec![(0, 3), (8, 2)]
        );
    }

    #[test]
    fn replace_into_an_empty_tree_places_the_leaf() {
        let mut tree = Tree::new();
        let leaf = tree.alloc_leaf(7, 5);
        tree.replace(0, 2, leaf);
        assert_eq!(tree.len, 5);
        assert_eq!(tree.root, leaf);
    }

    #[test]
    fn cut_removes_a_range_from_the_middle() {
        let mut tree = Tree::with_leaf(0, 10);
        tree.cut(3, 7);
        assert_eq!(tree.len, 6);
        assert_eq!(
            tree.leaves_in_order()
                .iter()
                .map(|l| (l.start, l.len))
                .collect::<Vec<_>>(),
            vec![(0, 3), (7, 3)]
        );
    }

    #[test]
    fn cut_across_several_leaves() {
        let mut tree = pieced_tree(&[(0, 3), (3, 3), (6, 3)]);
        tree.cut(2, 7);
        assert_eq!(tree.len, 4);
        assert_eq!(
            tree.leaves_in_order()
                .iter()
                .map(|l| (l.start, l.len))
                .collect::<Vec<_>>(),
            vec![(0, 2), (7, 2)]
        );
    }

    #[test]
    fn cut_ignores_an_empty_or_broken_range() {
        let mut tree = Tree::with_leaf(0, 4);
        tree.cut(3, 3);
        tree.cut(0, 0);
        assert_eq!(tree.len, 4, "пустой диапазон ничего не режет");
        let mut empty = Tree::new();
        empty.cut(0, 4);
        assert_eq!(empty.len, 0);
    }

    #[test]
    fn reading_returns_the_whole_buffer() {
        let tree = pieced_tree(&[(0, 3), (100, 2), (3, 4)]);
        let buffer = sample_buffer(120);
        let mut reader = RangeReader::new(&buffer);
        tree.read_into(&mut reader);
        let expected: Vec<u8> = [(0, 3), (100, 2), (3, 4)]
            .iter()
            .flat_map(|&(start, len)| buffer[start as usize..(start + len) as usize].to_vec())
            .collect();
        assert_eq!(reader.text, expected);
        assert_eq!(reader.ranges, vec![(0, 3), (100, 2), (3, 4)]);
    }

    #[test]
    fn reading_a_range_clips_to_the_bounds() {
        let tree = Tree::with_leaf(0, 10);
        let buffer = sample_buffer(10);
        let mut reader = RangeReader::new(&buffer);
        tree.read_range_into(3, 7, &mut reader);
        assert_eq!(reader.text, buffer[3..7].to_vec());
    }

    #[test]
    fn reading_an_empty_range_gives_nothing() {
        let tree = Tree::with_leaf(0, 10);
        let buffer = sample_buffer(10);
        let mut reader = RangeReader::new(&buffer);
        tree.read_range_into(5, 5, &mut reader);
        assert!(reader.text.is_empty());
        assert!(reader.ranges.is_empty());
    }

    #[test]
    fn reading_a_range_beyond_the_end_is_clipped() {
        let tree = Tree::with_leaf(0, 4);
        let buffer = sample_buffer(4);
        let mut reader = RangeReader::new(&buffer);
        tree.read_range_into(2, 100, &mut reader);
        assert_eq!(reader.text, buffer[2..4].to_vec());
    }

    #[test]
    fn leaves_in_order_lists_every_piece() {
        let tree = pieced_tree(&[(0, 3), (10, 3), (20, 2)]);
        let pieces: Vec<_> = tree
            .leaves_in_order()
            .iter()
            .map(|leaf| (leaf.start, leaf.len))
            .collect();
        assert_eq!(pieces, vec![(0, 3), (10, 3), (20, 2)]);
    }

    #[test]
    fn leaf_end_is_the_farthest_buffer_byte() {
        let tree = pieced_tree(&[(0, 3), (3, 3), (10, 2)]);
        assert_eq!(tree.leaf_end(), 12);
        let empty = Tree::new();
        assert_eq!(empty.leaf_end(), 0);
    }

    #[test]
    fn staying_balanced_under_many_inserts() {
        // Кусочки с разрывами: смежные слились бы в один лист, и балансировать
        // было бы нечего.
        let mut tree = Tree::new();
        for i in 0..500u32 {
            let leaf = tree.alloc_leaf(i * 4, 2);
            tree.insert_leaf(tree.len, leaf);
        }
        let stats = tree.debug_stats().expect("дерево не зациклено");
        assert_eq!(stats.height, tree.height());
        assert!(
            stats.worst_imbalance <= 1,
            "перекос {}",
            stats.worst_imbalance
        );
        assert_eq!(stats.height_mismatch, 0, "высота в узле разошлась");
        assert!(tree.debug_sound());
        assert_eq!(stats.nodes, tree.leaves * 2 - 1);
    }

    #[test]
    fn debug_stats_on_an_empty_tree() {
        let stats = Tree::new().debug_stats().expect("дерево не зациклено");
        assert_eq!(stats.nodes, 0);
        assert_eq!(stats.height, 0);
        assert_eq!(stats.worst_imbalance, 0);
        assert_eq!(stats.height_mismatch, 0);
    }

    #[test]
    fn debug_sound_notices_a_cycle() {
        // Собирается руками: узел ссылается сам на себя. Такого быть не
        // должно, и проверка обязана это заметить, а не зациклиться.
        let mut tree = Tree::new();
        let leaf = tree.alloc_leaf(0, 2);
        let branch = tree.make_branch(leaf, leaf);
        tree.root = branch;
        tree.nodes[branch as usize] = Node::branch(branch, branch, 4, 1);
        assert!(!tree.debug_sound());
        assert_eq!(tree.debug_reachable_nodes(), None);
        assert_eq!(tree.debug_stats(), None, "зацикленное дерево — без сводки");
    }

    #[test]
    fn node_at_and_node_text_len_describe_the_leaf() {
        let mut tree = Tree::new();
        let leaf = tree.alloc_leaf(9, 4);
        let node = tree.node_at(leaf);
        assert_eq!(node.start(), 9);
        assert_eq!(node.leaf_len(), 4);
        assert_eq!(tree.node_text_len(leaf), 4);
    }
}

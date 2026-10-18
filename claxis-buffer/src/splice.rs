//! Сборка дерева: склейка, вставка, замена, вырезание и балансировка.
//!
//! Вынесено из `tree.rs`, чтобы тот не рос за тысячу строк. Там остались пул,
//! выделение узлов и чтение; здесь — всё, что перестраивает структуру.

use super::Tree;
use crate::node::{NIL, Node};

impl Tree {
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
        // Оба узла читаются один раз: вид, высота, границы и дети берутся из
        // одного обращения к пулу, а не из трёх.
        let left_node = self.nodes[left as usize];
        let right_node = self.nodes[right as usize];
        if left_node.is_leaf()
            && right_node.is_leaf()
            && right_node.start() == left_node.start().saturating_add(left_node.leaf_len())
        {
            return self.merge_adjacent_leaves(left, right);
        }
        let (left_height, right_height) = (left_node.height(), right_node.height());
        if left_height > right_height + 1 {
            let middle = self.concat(left_node.right(), right);
            let joined = self.make_branch(left_node.left(), middle);
            self.release(left);
            // Свежий узел после спуска по правому ребёнку может разъехаться
            // на два: классический join обязан повернуть на каждом уровне, а
            // не только на верхнем. Без этого перекос копится в глубине.
            self.rebalance(joined)
        } else if right_height > left_height + 1 {
            let middle = self.concat(left, right_node.left());
            let joined = self.make_branch(middle, right_node.right());
            self.release(right);
            self.rebalance(joined)
        } else {
            self.make_branch(left, right)
        }
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
        if node.is_leaf() {
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
        // Длина листа читается один раз: она нужна и на проверку «пустой», и
        // на прирост длины документа.
        let gained = self.node_text_len(leaf);
        // Пустой лист ничего не вставляет: только вырезается диапазон.
        if gained == 0 {
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
        // Узел читается один раз: из него и длина поддерева, и вид, и дети.
        let node = self.nodes[id as usize];
        let subtree_end = subtree_start + self.text_len_of(&node);
        if end <= subtree_start || start >= subtree_end {
            return id;
        }
        if node.is_leaf() {
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
        // Узел читается один раз: из него и длина поддерева, и вид, и дети.
        let node = self.nodes[id as usize];
        let subtree_end = subtree_start + self.text_len_of(&node);
        if end <= subtree_start || start >= subtree_end {
            return id;
        }
        if node.is_leaf() {
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
            // `left_right` читается один раз: и на проверку «лист ли», и на
            // высоту, и на детей.
            let left_right_node = self.nodes[left_right as usize];
            // `(left_left, left_right, right)`: правый поворот даёт `left_left`
            // и `(left_right, right)`. Если перевесил правый ребёнок
            // `left_right`, сперва выправляется он, иначе перекос останется,
            // только переедет на уровень выше.
            if !left_right_node.is_leaf() && left_right_node.height() > self.node_height(left_left)
            {
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
            let right_left_node = self.nodes[right_left as usize];
            if !right_left_node.is_leaf()
                && right_left_node.height() > self.node_height(right_right)
            {
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
}

#[cfg(test)]
mod tests {
    use super::Tree;

    /// Генератор чисел: детерминированный, чтобы падение воспроизводилось.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u32) -> u32 {
            if n == 0 {
                return 0;
            }
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as u32
        }
    }

    /// Проверить устройство дерева после перестройки.
    ///
    /// Высоты, перекос, полнота и покрытие текста листьями: эти поломки не
    /// видны по одному чтению, а потом всплывают медленным спуском.
    fn assert_sound(tree: &Tree, what: &str) {
        let stats = tree.debug_stats().expect("дерево не зациклено");
        assert!(tree.debug_sound(), "{what}: дерево не связно");
        assert_eq!(stats.height_mismatch, 0, "{what}: высота в узле разошлась");
        assert_eq!(
            stats.height,
            tree.height(),
            "{what}: высота корня не сошлась"
        );
        assert!(
            stats.worst_imbalance <= 1,
            "{what}: перекос {}",
            stats.worst_imbalance
        );

        let leaves = tree.leaves_in_order();
        let total: u64 = leaves.iter().map(|leaf| leaf.len as u64).sum();
        assert_eq!(total, tree.len as u64, "{what}: листья не покрывают текст");
        if leaves.is_empty() {
            assert_eq!(stats.nodes, 0, "{what}: пустое дерево с узлами");
        } else {
            assert_eq!(
                stats.nodes,
                leaves.len() * 2 - 1,
                "{what}: дерево не полное"
            );
        }
    }

    #[test]
    fn the_tree_stays_sound_under_random_splices() {
        // Прямая нагрузка на сборку: вставка, замена и вырезание вперемешку с
        // произвольными диапазонами. Куски берутся из растущего буфера, чтобы
        // смежные листья не сливались сами собой и балансировке было что
        // делать. После каждой операции дерево обязано остаться связным,
        // сбалансированным и ровно покрыть текст.
        for seed in 0..16u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545F4914F6CDD1D) | 1);
            let mut tree = Tree::new();
            let mut watermark = 0u32;
            for step in 0..600 {
                let what = format!("seed {seed} step {step}");
                let len = tree.len;
                match rng.below(10) {
                    0..=3 if len > 0 => {
                        let start = rng.below(len);
                        let end = start + 1 + rng.below(len - start);
                        tree.cut(start, end);
                    }
                    4..=6 => {
                        let pos = rng.below(len + 1);
                        let take = 1 + rng.below(8);
                        let leaf = tree.alloc_leaf(watermark, take);
                        watermark += take;
                        tree.insert_leaf(pos, leaf);
                    }
                    _ => {
                        let pos = rng.below(len + 1);
                        let removed = rng.below((len - pos).min(12));
                        let take = 1 + rng.below(8);
                        let leaf = tree.alloc_leaf(watermark, take);
                        watermark += take;
                        tree.replace(pos, removed, leaf);
                    }
                }
                assert_sound(&tree, &what);
            }
        }
    }
}

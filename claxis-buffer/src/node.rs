//! Узел rope.
//!
//! Ровно **12 байт**: три `u32` подряд, без хвоста выравнивания. Признак листа
//! и высота ветки лежат не в узле, а рядом с ним, отдельным байтом в дереве.
//! Так индекс ребёнка занимает все 32 бита: в самом узле на признак места нет,
//! а урезать индекс ради него нельзя.
//!
//! Лист — это `start, len`: диапазон в общем буфере текста. Ни записи, ни
//! позиции вставки: лист не привязан к границам правок и потому не
//! разрезается при каждой из них. Это и даёт экономию — на дереве с записями
//! одна вставка в середину даёт три листа, здесь один.

/// Узел: лист или ветка.
///
/// Вид узла знает дерево: оно держит рядом байт с признаком листа и высотой.
///
/// Ветка: `a` — индекс левого ребёнка, `b` — правого, `c` — длина поддерева.
/// Лист: `a` — начало диапазона в буфере, `b` — длина, `c` не используется.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Node {
    a: u32,
    b: u32,
    c: u32,
}

/// Узел не существует.
pub const NIL: u32 = 0;

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({}, {}, {})", self.a, self.b, self.c)
    }
}

impl Node {
    /// Создать узел без вида: вид задаёт дерево.
    pub const fn new(a: u32, b: u32, c: u32) -> Self {
        Self { a, b, c }
    }

    /// Лист: диапазон буфера.
    pub const fn leaf(start: u32, len: u32) -> Self {
        Node {
            a: start,
            b: len,
            c: 0,
        }
    }

    /// Ветка: дети и длина поддерева.
    pub const fn branch(left: u32, right: u32, len: u32) -> Self {
        Node {
            a: left,
            b: right,
            c: len,
        }
    }

    /// Размер узла в байтах.
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    /// Откуда начинается текст листа.
    #[inline]
    pub const fn start(&self) -> u32 {
        self.a
    }

    /// Сколько байт в листе.
    #[inline]
    pub const fn leaf_len(&self) -> u32 {
        self.b
    }

    /// Следующий свободный узел в списке.
    #[inline]
    pub const fn free_next(&self) -> u32 {
        self.a
    }

    /// Индекс левого ребёнка.
    #[inline]
    pub const fn left(&self) -> u32 {
        self.a
    }

    #[inline]
    pub const fn right(&self) -> u32 {
        self.b
    }

    /// Длина поддерева, если это ветка.
    #[inline]
    pub const fn subtree_len(&self) -> u32 {
        self.c
    }
}

/// Узел свободен.
pub const FREE: Node = Node { a: NIL, b: 0, c: 0 };

/// Диапазон листа: откуда в буфере и сколько байт.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Leaf {
    /// Начало диапазона в буфере.
    pub start: u32,
    /// Длина диапазона.
    pub len: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leaf_keeps_the_offset_above_two_gigabytes() {
        // Смещение в буфере занимает все 32 бита: документ честно держит все
        // четыре гигабайта, а не два.
        for start in [0, 1, u32::MAX / 2, i32::MAX as u32 + 1, u32::MAX] {
            let leaf = Node::leaf(start, 7);
            assert_eq!(leaf.start(), start, "смещение {start} не уцелело");
            assert_eq!(leaf.leaf_len(), 7);
        }
    }

    #[test]
    fn a_node_is_twelve_bytes() {
        // Ровно три `u32`: ни байта на метку, ни хвоста выравнивания.
        assert_eq!(Node::size(), 12);
        assert_eq!(std::mem::size_of::<Node>(), 12);
    }

    #[test]
    fn new_node_carries_the_raw_fields() {
        let node = Node::new(4, 5, 6);
        assert_eq!(node.free_next(), 4);
        assert_eq!(node.left(), 4);
        assert_eq!(node.right(), 5);
        assert_eq!(node.subtree_len(), 6);
    }

    #[test]
    fn the_free_node_points_nowhere() {
        assert_eq!(FREE.free_next(), NIL);
        assert_eq!(FREE.leaf_len(), 0);
        assert_eq!(FREE.subtree_len(), 0);
    }

    #[test]
    fn debug_shows_the_three_fields() {
        let node = Node::branch(1, 2, 3);
        assert_eq!(format!("{node:?}"), "Node(1, 2, 3)");
    }

    #[test]
    fn a_leaf_range_keeps_its_bounds() {
        let leaf = Leaf { start: 9, len: 4 };
        assert_eq!(leaf.start, 9);
        assert_eq!(leaf.len, 4);
    }

    #[test]
    fn leaf_and_branch_keep_their_fields() {
        let leaf = Node::leaf(100, 20);
        assert_eq!(leaf.start(), 100);
        assert_eq!(leaf.leaf_len(), 20);

        let branch = Node::branch(3, 4, 120);
        assert_eq!(branch.left(), 3);
        assert_eq!(branch.right(), 4);
        assert_eq!(branch.subtree_len(), 120);
    }

    #[test]
    fn the_whole_left_child_index_survives() {
        // Раньше индекс левого ребёнка делил поле с высотой и меткой листа и
        // обрезался до двадцати четырёх битов. Теперь он занимает все 32.
        let branch = Node::branch(u32::MAX, u32::MAX - 1, u32::MAX);
        assert_eq!(branch.left(), u32::MAX);
        assert_eq!(branch.right(), u32::MAX - 1);
        assert_eq!(branch.subtree_len(), u32::MAX);
    }
}

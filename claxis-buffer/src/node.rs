//! Узел rope.
//!
//! Ровно **12 байт**: три `u32` подряд, без хвоста выравнивания. Вид узла
//! определяется старшим битом `a`, а не дискриминантом `enum`: обычный
//! `enum` стоил бы 16 байт, потому что четыре ушли бы на метку, которая здесь
//! не нужна — индексы узлов меньше `2^31`, и бит свободен.
//!
//! Лист — это `start, len`: диапазон в общем буфере текста. Ни записи, ни
//! позиции вставки: лист не привязан к границам правок и потому не
//! разрезается при каждой из них. Это и даёт экономию — на дереве с записями
//! одна вставка в середину даёт три листа, здесь один.

/// Старший бит `a`: узел-лист. Узлы-ветки держат в `a` индекс ребёнка.
const LEAF: u32 = 1 << 31;

/// Высота упакована в биты `a` над индексом левого ребёнка.
///
/// Семь бит, а не восемь: старший бит `a` занят меткой листа, и высота не
/// имеет права его занимать — иначе ветка со ста уровнями выглядела бы
/// листом. 127 уровней хватает с запасом: столько узлов не набирает даже
/// rope на сотни миллионов байт.
const HEIGHT_SHIFT: u32 = 24;
/// Младшие биты `a` — индекс левого ребёнка. Индекс занимает двадцать четыре
/// бита, а не все тридцать два: остальные делят поле с высотой и меткой. Это
/// потолок в шестнадцать с лишним миллионов узлов, и на дерево с записями его
/// хватает с избытком.
const INDEX_MASK: u32 = (1 << HEIGHT_SHIFT) - 1;
/// Биты высоты: всё, кроме метки листа.
const HEIGHT_MASK: u32 = 0b0111_1111;

/// Узел свободен: поле листа обнулено, чтобы не держать старый диапазон.
const DEAD: Node = Node {
    a: LEAF,
    b: 0,
    c: 0,
};

/// Узел: лист или ветка.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Node {
    /// Ветка: высота над индексом левого ребёнка. Лист: `LEAF | start`.
    a: u32,
    /// Ветка: индекс правого ребёнка. Лист: длина диапазона.
    b: u32,
    /// Ветка: длина поддерева. Лист: старший бит смещения в буфере.
    c: u32,
}

/// Узел не существует.
pub const NIL: u32 = 0;

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_leaf() {
            write!(f, "Leaf({}, {})", self.start(), self.leaf_len())
        } else {
            write!(
                f,
                "Branch {{ left: {}, right: {}, len: {} }}",
                self.left(),
                self.right(),
                self.subtree_len()
            )
        }
    }
}

impl Node {
    /// Свободный узел: поле `a` держит индекс следующего свободного.
    ///
    /// Своим видом `is_leaf` не считается: индексы узлов не выходят за 24 бита,
    /// поэтому метка листа в старшем бите не появляется, и `free_next()` читает
    /// индекс верно. Ровно так свободные узлы и связываются в список.
    pub const fn free(next: u32) -> Self {
        Self {
            a: next,
            b: 0,
            c: 0,
        }
    }

    /// Создать узел из сырых полей. Только для проверок.
    ///
    /// Вид узла задаёт старший бит `a`: сырое значение с этим битом даёт лист с
    /// мусорным `free_next()`. Обычный код зовёт [`Node::leaf`], [`Node::branch`]
    /// и [`Node::free`], где вид выставлен по делу.
    #[cfg(test)]
    pub const fn new(a: u32, b: u32, c: u32) -> Self {
        Self { a, b, c }
    }

    /// Лист: диапазон буфера.
    pub const fn leaf(start: u32, len: u32) -> Self {
        Node {
            // Старший бит смещения не влезает в `a` — там метка листа. Он
            // лежит в `c`: у листа это поле свободно, а документ честно держит
            // все четыре гигабайта, а не два.
            a: LEAF | (start & !LEAF),
            b: len,
            c: start >> 31,
        }
    }

    /// Ветка: дети, длина и высота.
    ///
    /// Высота лежит рядом с левым ребёнком, а не в четвёртом поле: узла на
    /// четыре `u32` не было бы, а хранить её надо — иначе баланс приходится
    /// проверять обходом поддерева, и вставка становится квадратичной.
    pub const fn branch(left: u32, right: u32, len: u32, height: u32) -> Self {
        Self {
            a: ((height & HEIGHT_MASK) << HEIGHT_SHIFT) | (left & INDEX_MASK),
            b: right,
            c: len,
        }
    }

    /// Размер узла в байтах.
    pub const fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    /// Лист ли узел.
    #[inline]
    pub const fn is_leaf(&self) -> bool {
        self.a & LEAF != 0
    }

    /// Откуда начинается текст листа.
    #[inline]
    pub const fn start(&self) -> u32 {
        (self.c << 31) | (self.a & !LEAF)
    }

    /// Сколько байт в листе.
    #[inline]
    pub const fn leaf_len(&self) -> u32 {
        self.b
    }

    /// Следующий свободный узел в списке: поле `a` без метки листа.
    #[inline]
    pub const fn free_next(&self) -> u32 {
        self.a & !LEAF
    }

    /// Индекс левого ребёнка.
    #[inline]
    pub const fn left(&self) -> u32 {
        self.a & INDEX_MASK
    }

    /// Высота поддерева: лист — единица.
    #[inline]
    pub const fn height(&self) -> u32 {
        if self.is_leaf() {
            1
        } else {
            (self.a >> HEIGHT_SHIFT) & HEIGHT_MASK
        }
    }

    /// Индекс правого ребёнка.
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
pub const FREE: Node = DEAD;

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
        // Старший бит смещения лежит в свободном поле листа. Если он потеряется,
        // документ больше двух гигабайт прочитает чужой кусок — молча.
        for start in [0, 1, u32::MAX / 2, i32::MAX as u32 + 1, u32::MAX] {
            let leaf = Node::leaf(start, 7);
            assert!(leaf.is_leaf());
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
    fn debug_tells_a_leaf_from_a_branch() {
        let leaf = Node::leaf(100, 20);
        assert_eq!(format!("{leaf:?}"), "Leaf(100, 20)");
        let branch = Node::branch(1, 2, 3, 4);
        assert_eq!(
            format!("{branch:?}"),
            "Branch { left: 1, right: 2, len: 3 }"
        );
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
        assert!(leaf.is_leaf());
        assert_eq!(leaf.start(), 100);
        assert_eq!(leaf.leaf_len(), 20);
        assert_eq!(leaf.height(), 1);

        let branch = Node::branch(3, 4, 120, 7);
        assert!(!branch.is_leaf());
        assert_eq!(branch.left(), 3);
        assert_eq!(branch.right(), 4);
        assert_eq!(branch.subtree_len(), 120);
        assert_eq!(branch.height(), 7);
    }

    #[test]
    fn the_left_index_keeps_twenty_four_bits() {
        // Индекс левого ребёнка делит поле с высотой и меткой листа, поэтому
        // больше двадцати четырёх бит он не занимает.
        let branch = Node::branch(u32::MAX, u32::MAX - 1, u32::MAX, 127);
        assert_eq!(branch.left(), INDEX_MASK);
        assert_eq!(branch.right(), u32::MAX - 1);
        assert_eq!(branch.subtree_len(), u32::MAX);
        assert_eq!(branch.height(), 127);
        assert!(!branch.is_leaf(), "высота не задела метку листа");
    }
}

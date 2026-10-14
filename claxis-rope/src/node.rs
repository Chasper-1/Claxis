//! Узел rope.
//!
//! Ровно **12 байт**: три `u32` подряд, без хвоста выравнивания. Вид узла
//! определяется старшим битом `a`, а не дискриминантом `enum`: обычный `enum`
//! стоил бы 16 байт, потому что четыре ушли бы на метку, которая здесь не
//! нужна — индексы узлов меньше `2^31`, и бит свободен.
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
/// имеет права его занимать — иначе ветка со стом уровнями выглядела бы
/// листом. 127 уровней хватает с запасом: столько узлов не набирает даже
/// rope на сотни миллионов байт.
const HEIGHT_SHIFT: u32 = 24;
/// Младшие биты `a` — индекс левого ребёнка.
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
    /// Ветка: индекс левого ребёнка. Лист: `LEAF | start`.
    a: u32,
    /// Ветка: индекс правого ребёнка. Лист: длина диапазона.
    b: u32,
    /// Ветка: длина поддерева. Лист: не используется.
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
                self.len()
            )
        }
    }
}

impl Node {
    /// Создать узел.
    pub const fn new(a: u32, b: u32, c: u32) -> Self {
        Self { a, b, c }
    }

    /// Лист: диапазон буфера.
    pub const fn leaf(start: u32, len: u32) -> Self {
        Self {
            a: LEAF | start,
            b: len,
            c: 0,
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

    #[inline]
    pub const fn is_leaf(&self) -> bool {
        self.a & LEAF != 0
    }

    /// Откуда начинается текст листа.
    #[inline]
    pub const fn start(&self) -> u32 {
        self.a & !LEAF
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

    #[inline]
    pub const fn right(&self) -> u32 {
        self.b
    }

    /// Длина документа, если это ветка.
    #[inline]
    pub const fn len(&self) -> u32 {
        self.c
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.c == 0
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
    fn a_node_is_twelve_bytes() {
        // Ровно три `u32`: ни байта на метку, ни хвоста выравнивания.
        assert_eq!(Node::size(), 12);
        assert_eq!(std::mem::size_of::<Node>(), 12);
    }

    #[test]
    fn leaf_and_branch_tell_themselves_apart() {
        let leaf = Node::leaf(100, 20);
        assert!(leaf.is_leaf());
        assert_eq!(leaf.start(), 100);
        assert_eq!(leaf.leaf_len(), 20);

        let branch = Node::branch(3, 4, 120, 7);
        assert!(!branch.is_leaf());
        assert_eq!(branch.left(), 3);
        assert_eq!(branch.right(), 4);
        assert_eq!(branch.len(), 120);
        assert_eq!(branch.height(), 7);
        assert_eq!(Node::leaf(100, 20).height(), 1);
    }

    #[test]
    fn height_does_not_eat_the_left_child() {
        // Высота и индекс левого ребёнка делят одно поле: одно не должно
        // портитьть другое.
        let branch = Node::branch(1 << 20, 2, 10, 127);
        assert_eq!(branch.left(), 1 << 20);
        assert_eq!(branch.height(), 127);
        assert!(!branch.is_leaf(), "высота не задела метку листа");
    }
}

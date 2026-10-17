//! Узел rope: лист или ветка с детьми.

use crate::leaf::{Leaf, MAX_CHUNKS};
use crate::pool::Pool;

/// В каком отношении вес ребёнка может отставать от веса узла, прежде чем узел
/// повернётся.
pub const IMBALANCE: u32 = 4;

/// Сколько детей может быть у узла, прежде чем он разделится пополам.
pub const MAX_CHILDREN: usize = 4;

/// Узел разделяется пополам: нижняя половина детей и верхняя становятся двумя
/// детьми нового узла. Вес половин равен половине веса узла, поэтому баланс
/// держится по весу. Новые ветки берутся из пула.
pub fn halve(node: Node, nodes: &mut Pool<Node>) -> Option<Node> {
    let Node::Branch(mut branch) = node else {
        return None;
    };
    if branch.children.len() <= MAX_CHILDREN {
        return None;
    }
    let mid = branch.children.len() / 2;
    let upper = branch.children.split_off(mid);
    let lower = nodes.take(|| Node::Branch(Branch::new(Vec::new())));
    let upper = nodes.take(|| Node::Branch(Branch::new(upper)));
    Some(Node::Branch(Branch::new(vec![
        put(lower, branch.children),
        upper,
    ])))
}

/// Кладёт детей на место, отданное пулом.
fn put(node: Node, children: Vec<Node>) -> Node {
    match node {
        Node::Branch(mut branch) => {
            branch.children = children;
            branch.weigh();
            Node::Branch(branch)
        }
        leaf => leaf,
    }
}

/// Чем заменить узел, когда он уже есть в дереве: пустой лист.
const EMPTY: Node = Node::Leaf(Leaf::new(0, 0));

/// Ветка: дети и их суммарный вес.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub(crate) children: Vec<Node>,
    pub(crate) len: u32,
}

/// Узел rope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Лист: диапазон общего буфера.
    Leaf(Leaf),
    /// Ветка: несколько детей.
    Branch(Branch),
}

impl Branch {
    /// Ветка над `children`.
    pub fn new(children: Vec<Node>) -> Self {
        let len = children.iter().map(Node::len).sum();
        Self { children, len }
    }

    /// Дети ветки.
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// Дети ветки на запись: нужны спуску и разбору дерева.
    pub fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.children
    }

    /// Суммарный вес ветки.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Ветка пуста, если детей нет.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Пересчитывает вес по детям. Вес хранится, а не пересчитывается на лету.
    pub fn weigh(&mut self) {
        self.len = self.children.iter().map(Node::len).sum();
    }

    /// Повернуть ветку: первый ребёнок уходит вверх, его первый ребёнок — сюда.
    pub fn rotate_right(&mut self) -> Option<Node> {
        let first = std::mem::replace(&mut self.children[0], EMPTY);
        let Node::Branch(mut lifted) = first else {
            // Первый ребёнок — лист: поворачивать нечего.
            self.children[0] = first;
            return None;
        };
        if lifted.children.is_empty() {
            lifted.children.push(Node::Leaf(Leaf::new(0, 0)));
        }
        self.children[0] = lifted.children.remove(0);
        self.weigh();
        // Поднятая ветка весит и своих детей, и всё, что осталось здесь.
        let moved = self.children[0].len();
        lifted.len = self.len + moved;
        Some(Node::Branch(lifted))
    }

    /// Повернуть ветку: последний ребёнок уходит вверх, его последний ребёнок —
    /// сюда.
    pub fn rotate_left(&mut self) -> Option<Node> {
        let Some(Node::Branch(_)) = self.children.last() else {
            // Последний ребёнок — лист: поворачивать нечего.
            return None;
        };
        let Some(Node::Branch(mut lifted)) = self.children.pop() else {
            return None;
        };
        if lifted.children.is_empty() {
            lifted.children.push(Node::Leaf(Leaf::new(0, 0)));
        }
        self.children
            .push(lifted.children.pop().expect("у поднятой ветки есть дети"));
        self.weigh();
        // Поднятая ветка весит и своих детей, и всё, что осталось здесь.
        let moved = self.children[self.children.len() - 1].len();
        lifted.len = self.len + moved;
        Some(Node::Branch(lifted))
    }

    /// Поворот, если первый ребёнок слишком лёгкий.
    pub fn balance_right(&mut self) -> Option<Node> {
        if self.children.len() < 3 || self.children[0].len() * IMBALANCE > self.len {
            return None;
        }
        self.rotate_right()
    }

    /// Поворот, если последний ребёнок слишком лёгкий.
    pub fn balance_left(&mut self) -> Option<Node> {
        if self.children.len() < 3 {
            return None;
        }
        let last = self.children.len() - 1;
        if self.children[last].len() * IMBALANCE > self.len {
            return None;
        }
        self.rotate_left()
    }
}

impl Node {
    /// Вес узла: число байт под ним.
    pub fn len(&self) -> u32 {
        match self {
            Node::Leaf(leaf) => leaf.len(),
            Node::Branch(branch) => branch.len(),
        }
    }

    /// Узел пуст, если под ним нет ни байта.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Узел — лист.
    pub fn is_leaf(&self) -> bool {
        matches!(self, Node::Leaf(_))
    }

    /// Лист узла, если узел им является.
    pub fn leaf(&self) -> Option<Leaf> {
        match self {
            Node::Leaf(leaf) => Some(*leaf),
            Node::Branch(_) => None,
        }
    }
    /// Сливает первые двух детей в один. Съеденный лист отдаётся наверх: его
    /// вернёт в пул вызывающий. Если сливать нечего — `None`.
    pub fn merge_leaves(&mut self, chunk: usize) -> Option<Leaf> {
        let Node::Branch(branch) = self else {
            return None;
        };
        if branch.children.len() < 2 {
            return None;
        }
        let (mut first, second) = match (&branch.children[0], &branch.children[1]) {
            (Node::Leaf(first), Node::Leaf(second)) => (*first, *second),
            _ => return None,
        };
        if !first.absorb(&second, chunk) {
            return None;
        }
        branch.children[0] = Node::Leaf(first);
        branch.children.remove(1);
        branch.weigh();
        Some(second)
    }

    /// Сдвигает узел и всех его детей на `delta` байт.
    pub fn shift_by(&mut self, at: u32, delta: i64) {
        match self {
            Node::Leaf(leaf) => leaf.shift_by(at, delta),
            Node::Branch(branch) => {
                for child in branch.children_mut() {
                    child.shift_by(at, delta);
                }
            }
        }
    }

    /// Лист, который тянет больше двух чанков, делится по границе чанка. Наверх
    /// уходят номер листа и его правая половина: вставить её должен вызывающий,
    /// у него пул.
    pub fn split_leaf(&mut self, chunk: usize) -> Option<(usize, Leaf)> {
        let Node::Branch(branch) = self else {
            return None;
        };
        for index in 0..branch.children.len() {
            if let Node::Leaf(leaf) = branch.children[index]
                && leaf.chunks(chunk) > MAX_CHUNKS
            {
                let (left, right) = leaf.split(chunk);
                branch.children[index] = Node::Leaf(left);
                branch.weigh();
                return Some((index, right));
            }
        }
        None
    }

    /// Поворот вверх, если первый ребёнок слишком лёгкий.
    pub fn balance_right(&mut self) -> Option<Node> {
        let Node::Branch(branch) = self else {
            return None;
        };
        branch.balance_right()
    }

    /// Поворот вверх, если последний ребёнок слишком лёгкий.
    pub fn balance_left(&mut self) -> Option<Node> {
        let Node::Branch(branch) = self else {
            return None;
        };
        branch.balance_left()
    }

    /// Лист, который тянет больше двух чанков, становится веткой из двух листьев.
    /// Наверх уходит новая ветка.
    pub fn wrap(&self, chunk: usize) -> Option<Node> {
        let Node::Leaf(leaf) = self else {
            return None;
        };
        if leaf.chunks(chunk) <= MAX_CHUNKS {
            return None;
        }
        let (left, right) = leaf.split(chunk);
        Some(Node::Branch(Branch::new(vec![
            Node::Leaf(left),
            Node::Leaf(right),
        ])))
    }

    /// Ставит лист на место `index`.
    pub fn add_leaf(&mut self, index: usize, leaf: Leaf) {
        let Node::Branch(branch) = self else {
            return;
        };
        branch.children.insert(index, Node::Leaf(leaf));
        branch.weigh();
    }

    /// Убирает пустой лист: после вырезания он больше ничего не держит. Наверх
    /// уходят номер листа и он сам.
    pub fn remove_empty_leaf(&mut self) -> Option<(usize, Leaf)> {
        let Node::Branch(branch) = self else {
            return None;
        };
        if branch.children.len() < 2 {
            // Последний пустой лист убирать нельзя: ветка останется без детей.
            return None;
        }
        let index = branch.children.iter().position(|child| match child {
            Node::Leaf(leaf) => leaf.is_empty(),
            Node::Branch(_) => false,
        })?;
        let Node::Leaf(leaf) = branch.children.remove(index) else {
            return None;
        };
        branch.weigh();
        Some((index, leaf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_weighs_its_children() {
        let branch = Branch::new(vec![
            Node::Leaf(Leaf::new(0, 10)),
            Node::Leaf(Leaf::new(10, 30)),
        ]);
        assert_eq!(branch.len(), 30);
    }

    #[test]
    fn rotation_lifts_a_light_branch_out() {
        let mut branch = Branch::new(vec![
            Node::Branch(Branch::new(vec![
                Node::Leaf(Leaf::new(0, 1)),
                Node::Leaf(Leaf::new(1, 2)),
            ])),
            Node::Leaf(Leaf::new(2, 42)),
            Node::Leaf(Leaf::new(42, 78)),
        ]);
        let lifted = branch.balance_right().expect("поворот");
        assert_eq!(branch.len(), 77);
        assert_eq!(lifted.len(), 78);
    }

    #[test]
    fn a_leaf_child_is_not_rotated() {
        let mut branch = Branch::new(vec![
            Node::Leaf(Leaf::new(0, 1)),
            Node::Leaf(Leaf::new(1, 42)),
            Node::Leaf(Leaf::new(42, 78)),
        ]);
        assert!(branch.balance_right().is_none());
    }

    #[test]
    fn nothing_to_rotate_while_the_first_child_is_heavy() {
        let mut branch = Branch::new(vec![
            Node::Leaf(Leaf::new(0, 50)),
            Node::Leaf(Leaf::new(50, 60)),
            Node::Leaf(Leaf::new(60, 70)),
        ]);
        assert!(branch.balance_right().is_none());
        assert_eq!(branch.len(), 70);
    }

    #[test]
    fn adjacent_leaves_merge() {
        let mut branch = Node::Branch(Branch::new(vec![
            Node::Leaf(Leaf::new(0, 512)),
            Node::Leaf(Leaf::new(512, 1024)),
        ]));
        let absorbed = branch.merge_leaves(512).expect("листья сливаются");
        let Node::Branch(branch) = &branch else {
            unreachable!("ветка осталась веткой");
        };
        assert_eq!(branch.len(), 1024);
        assert_eq!(branch.children(), &[Node::Leaf(Leaf::new(0, 1024))]);
        assert_eq!(absorbed, Leaf::new(512, 1024));
    }

    #[test]
    fn a_leaf_of_five_chunks_splits() {
        let mut branch = Node::Branch(Branch::new(vec![Node::Leaf(Leaf::new(0, 5 * 512))]));
        let (index, right) = branch.split_leaf(512).expect("лист делится");
        branch.add_leaf(index + 1, right);
        assert_eq!(index, 0);
        assert_eq!(right, Leaf::new(2 * 512, 5 * 512));
        let Node::Branch(branch) = &branch else {
            unreachable!("ветка осталась веткой");
        };
        assert_eq!(branch.len(), 5 * 512);
        assert_eq!(branch.children().len(), 2);
    }

    #[test]
    fn an_empty_leaf_leaves_the_branch_only_with_a_sibling() {
        let mut branch = Node::Branch(Branch::new(vec![Node::Leaf(Leaf::new(0, 0))]));
        assert!(branch.remove_empty_leaf().is_none());

        let mut branch = Node::Branch(Branch::new(vec![
            Node::Leaf(Leaf::new(0, 0)),
            Node::Leaf(Leaf::new(0, 10)),
        ]));
        let (index, leaf) = branch.remove_empty_leaf().expect("пустой лист убран");
        assert_eq!(index, 0);
        assert_eq!(leaf, Leaf::new(0, 0));
        let Node::Branch(branch) = &branch else {
            unreachable!("ветка осталась веткой");
        };
        assert_eq!(branch.len(), 10);
    }
}

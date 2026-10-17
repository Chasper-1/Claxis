//! Дерево rope: листья над общим буфером, баланс по весу.

use crate::chunk::chunk_size;
use crate::leaf::{Leaf, MAX_CHUNKS};
use crate::node::{self, Branch, Node};
use crate::pool::Pool;

/// Лист пуст: им подменяется узел на время разбора дерева.
const EMPTY: Node = Node::Leaf(Leaf::new(0, 0));

/// Дерево листьев над общим буфером.
#[derive(Debug)]
pub struct Tree {
    root: Node,
    chunk: usize,
    nodes: Pool<Node>,
}

impl Tree {
    /// Дерево над `len` байт общего буфера. Листья режутся по чанкам сразу: большой
    /// файл не должен начинаться с одного листа на всю длину.
    pub fn new(len: u32) -> Self {
        let chunk = chunk_size(len as usize);
        let mut leaves = Vec::new();
        let mut start = 0;
        while start < len {
            let end = (start + (MAX_CHUNKS * chunk) as u32).min(len);
            leaves.push(Leaf::new(start, end));
            start = end;
        }
        if leaves.is_empty() {
            leaves.push(Leaf::new(0, 0));
        }
        Self {
            root: Self::pack(&leaves),
            chunk,
            nodes: Pool::new(),
        }
    }

    /// Складывает листья в дерево пополам: половина детей — половина веса.
    fn pack(leaves: &[Leaf]) -> Node {
        match leaves {
            [] => Node::Leaf(Leaf::new(0, 0)),
            [one] => Node::Leaf(*one),
            many => {
                let mid = many.len() / 2;
                Node::Branch(Branch::new(vec![
                    Self::pack(&many[..mid]),
                    Self::pack(&many[mid..]),
                ]))
            }
        }
    }

    /// Размер чанка, по которому режутся листья.
    pub fn chunk(&self) -> usize {
        self.chunk
    }

    /// Число байт под деревом.
    pub fn len(&self) -> u32 {
        self.root.len()
    }

    /// Дерево пусто, если под ним нет ни байта.
    pub fn is_empty(&self) -> bool {
        self.root.is_empty()
    }

    /// Корень дерева: нужен для проверок и для замеров.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Вставляет `len` байт в позицию `at`.
    pub fn insert(&mut self, at: u32, len: u32) {
        if len == 0 {
            return;
        }
        let root = std::mem::replace(&mut self.root, EMPTY);
        self.root = self.walk(root, at, len, 0);
        self.rechunk();
    }

    /// Вырезает `len` байт начиная с позиции `at`.
    pub fn delete(&mut self, at: u32, len: u32) {
        if len == 0 {
            return;
        }
        let root = std::mem::replace(&mut self.root, EMPTY);
        // Вырезание может накрыть много листьев сразу, поэтому идём по всему
        // дереву, а не спускаемся к одному листу.
        let root = Self::cut(root, at, len).unwrap_or(Node::Leaf(Leaf::new(0, 0)));
        self.root = root;
        self.rechunk();
        // Листья могли остаться соседями, которые вместе влезают в два чанка:
        // слить их можно только заново, соседи лежат в разных ветках.
        self.root = self.compact();
    }

    /// Вырезает диапазон из узла. Узел, целиком попавший в диапазон, пропадает.
    fn cut(node: Node, at: u32, len: u32) -> Option<Node> {
        let last = at + len;
        match node {
            Node::Leaf(mut leaf) => {
                if leaf.end <= at {
                    // лист целиком до диапазона
                    return Some(Node::Leaf(leaf));
                }
                if leaf.start >= last {
                    // лист целиком после диапазона: сдвигается влево
                    leaf.shift_by(at, -(len as i64));
                    return Some(Node::Leaf(leaf));
                }
                if leaf.start >= at && leaf.end <= last {
                    // лист целиком в диапазоне: его больше нет
                    return None;
                }
                if leaf.start < at {
                    // окно сидит в листе или режет его хвост
                    leaf.end = if leaf.end <= last { at } else { leaf.end - len };
                } else {
                    // окно режет начало листа: хвост сдвигается влево
                    leaf.start = at;
                    leaf.end -= len;
                }
                Some(Node::Leaf(leaf))
            }
            Node::Branch(mut branch) => {
                let mut kept = Vec::with_capacity(branch.children.len());
                for child in branch.children_mut().drain(..) {
                    if let Some(child) = Self::cut(child, at, len) {
                        kept.push(child);
                    }
                }
                if kept.is_empty() {
                    // ветка без детей — это пустой лист
                    return Some(Node::Leaf(Leaf::new(0, 0)));
                }
                branch.children = kept;
                branch.weigh();
                Some(Node::Branch(branch))
            }
        }
    }

    /// Сливает соседние листья и складывает дерево заново.
    fn compact(&mut self) -> Node {
        let mut found = Vec::new();
        Self::flatten(&self.root, &mut found);
        let mut merged: Vec<Leaf> = Vec::with_capacity(found.len());
        for leaf in found {
            let joins = match merged.last_mut() {
                Some(last) => last.absorb(&leaf, self.chunk),
                None => false,
            };
            if !joins {
                merged.push(leaf);
            }
        }
        Self::pack(&merged)
    }

    /// Раскладывает дерево в листья слева направо.
    fn flatten(node: &Node, out: &mut Vec<Leaf>) {
        match node {
            Node::Leaf(leaf) => out.push(*leaf),
            Node::Branch(branch) => {
                for child in branch.children() {
                    Self::flatten(child, out);
                }
            }
        }
    }

    /// Размер чанка зависит от объёма файла: после правки он мог смениться.
    fn rechunk(&mut self) {
        self.chunk = chunk_size(self.len() as usize);
    }

    /// Спускается до листа, отдаёт его вверх, правит дерево на обратном ходе.
    fn walk(&mut self, node: Node, at: u32, insert: u32, delete: u32) -> Node {
        let mut node = match node {
            Node::Leaf(mut leaf) => {
                if delete > 0 {
                    leaf.cut(at, delete);
                }
                if insert > 0 {
                    leaf.extend(at, insert);
                }
                return self.balance(Node::Leaf(leaf));
            }
            branch => branch,
        };
        if let Node::Branch(branch) = &mut node {
            let index = Self::child_at(branch, at);
            let offset = Self::offset(branch, at, index);
            let child = std::mem::replace(&mut branch.children_mut()[index], EMPTY);
            let child = self.walk(child, offset, insert, delete);
            branch.children_mut()[index] = child;
            // Правка сдвигает всё, что стоит правее: буфер-то общий.
            let delta = insert as i64 - delete as i64;
            if delta != 0 {
                for sibling in branch.children_mut()[index + 1..].iter_mut() {
                    sibling.shift_by(at, delta);
                }
            }
            branch.weigh();
        }
        self.balance(node)
    }

    /// Номер ребёнка, в котором лежит позиция `at`. Позиция за концом текста
    /// попадает в последнего ребёнка: вставка идёт в его конец.
    fn child_at(branch: &Branch, at: u32) -> usize {
        let mut left = at;
        let children = branch.children();
        for (index, child) in children.iter().enumerate() {
            if left < child.len() || index + 1 == children.len() {
                return index;
            }
            left -= child.len();
        }
        unreachable!("у ветки есть дети");
    }

    /// Позиция `at` внутри выбранного ребёнка.
    fn offset(branch: &Branch, at: u32, index: usize) -> u32 {
        at - branch.children()[..index]
            .iter()
            .map(Node::len)
            .sum::<u32>()
    }

    /// Делит, сливает и поворачивает узел, пока он не станет вменяемым.
    fn balance(&mut self, mut node: Node) -> Node {
        for _ in 0..4096 {
            if let Some(wrapped) = node.wrap(self.chunk) {
                node = wrapped;
                continue;
            }
            if let Some((index, right)) = node.split_leaf(self.chunk) {
                node.add_leaf(index + 1, right);
                continue;
            }
            if node.merge_leaves(self.chunk).is_some() {
                continue;
            }
            if node.remove_empty_leaf().is_some() {
                continue;
            }
            if let Some(lifted) = node::halve(node.clone(), &mut self.nodes) {
                node = lifted;
                continue;
            }
            return node;
        }
        panic!("баланс не сходится: {}", self.describe(&node));
    }

    /// Короткое описание узла: сколько детей и сколько они весят.
    fn describe(&self, node: &Node) -> String {
        match node {
            Node::Leaf(leaf) => format!("лист {leaf:?}"),
            Node::Branch(branch) => {
                let weights: Vec<u32> = branch.children().iter().map(Node::len).collect();
                format!(
                    "ветка {:?}, вес {}, чанк {}",
                    weights,
                    branch.len(),
                    self.chunk
                )
            }
        }
    }

    /// Ветка из пула: пул отдаёт место, дети заносятся в него.
    pub fn branch(&mut self, children: Vec<Node>) -> Node {
        let taken = self.nodes.take(|| Node::Branch(Branch::new(Vec::new())));
        match taken {
            Node::Branch(mut branch) => {
                branch.children = children;
                branch.weigh();
                Node::Branch(branch)
            }
            other => other,
        }
    }

    /// Сколько веток пул держит для переиспользования.
    pub fn free_nodes(&self) -> usize {
        self.nodes.free()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk;

    /// Раскладывает дерево в листья слева направо.
    fn leaves(node: &Node, out: &mut Vec<Leaf>) {
        match node {
            Node::Leaf(leaf) => out.push(*leaf),
            Node::Branch(branch) => {
                for child in branch.children() {
                    leaves(child, out);
                }
            }
        }
    }

    fn flat(tree: &Tree) -> Vec<Leaf> {
        let mut out = Vec::new();
        leaves(tree.root(), &mut out);
        out
    }

    #[test]
    fn a_new_tree_covers_the_buffer() {
        let tree = Tree::new(10);
        assert_eq!(tree.len(), 10);
        assert_eq!(tree.chunk(), chunk::SMALL);
        assert_eq!(flat(&tree), vec![Leaf::new(0, 10)]);
    }

    #[test]
    fn insertion_extends_one_leaf() {
        let mut tree = Tree::new(4);
        tree.insert(2, 6);
        assert_eq!(tree.len(), 10);
        assert_eq!(flat(&tree), vec![Leaf::new(0, 10)]);
    }

    #[test]
    fn a_leaf_over_two_chunks_is_split() {
        let chunk = chunk::SMALL as u32;
        let mut tree = Tree::new(chunk);
        tree.insert(0, 4 * chunk);
        assert_eq!(tree.chunk(), chunk_size(tree.len() as usize));
        assert_eq!(tree.len(), 5 * chunk);
        let leaves = flat(&tree);
        assert!(leaves.len() >= 2, "лист должен поделиться");
        for leaf in leaves {
            assert!(
                leaf.chunks(tree.chunk()) <= crate::leaf::MAX_CHUNKS,
                "лист {leaf:?} тянет больше двух чанков"
            );
        }
    }

    #[test]
    fn every_chunk_boundary_is_a_leaf_border() {
        let chunk = chunk::SMALL as u32;
        let mut tree = Tree::new(1);
        tree.insert(0, 10 * chunk);
        let leaves = flat(&tree);
        for (index, leaf) in leaves.iter().enumerate() {
            if leaf.chunks(tree.chunk()) == crate::leaf::MAX_CHUNKS && index + 1 < leaves.len() {
                assert_eq!(leaf.end() as usize % tree.chunk(), 0);
            }
        }
    }

    #[test]
    /// Глубина дерева растёт логарифмически от числа листьев, а не линейно.
    fn the_tree_stays_shallow() {
        let chunk = chunk::SMALL as u32;
        let mut tree = Tree::new(chunk);
        tree.insert(0, 200 * chunk);
        let mut found = Vec::new();
        leaves(tree.root(), &mut found);
        let leaves = found.len();
        let depth = depth(tree.root());
        assert!(
            depth * 2 <= leaves,
            "{leaves} листьев при глубине {depth}: дерево вырождено"
        );
    }

    #[test]
    fn deletion_shrinks_the_leaf() {
        let mut tree = Tree::new(10);
        tree.delete(3, 4);
        assert_eq!(tree.len(), 6);
        assert_eq!(flat(&tree), vec![Leaf::new(0, 6)]);
    }

    #[test]
    fn deleting_past_the_end_takes_what_is_left() {
        let mut tree = Tree::new(10);
        tree.delete(8, 100);
        assert_eq!(tree.len(), 8);
    }

    #[test]
    fn neighbours_merge_again() {
        let chunk = chunk::SMALL as u32;
        let mut tree = Tree::new(1);
        tree.insert(0, 10 * chunk);
        let split = flat(&tree).len();
        tree.delete(0, 9 * chunk - 1);
        assert_eq!(tree.len(), 10 * chunk + 1 - (9 * chunk - 1));
        assert!(flat(&tree).len() < split, "листья должны слиться");
    }

    /// Глубина дерева.
    fn depth(node: &Node) -> usize {
        match node {
            Node::Leaf(_) => 1,
            Node::Branch(branch) => 1 + branch.children().iter().map(depth).max().unwrap_or(0),
        }
    }
}

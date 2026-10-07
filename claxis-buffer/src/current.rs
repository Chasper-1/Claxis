use crate::arena::{AddArena, AddId};
use crate::segment::{Segment, Source};

pub(crate) type NodeId = u32;

/// Узел — только ссылки: источник, смещение, длина, следующий узел.
/// Записей истории здесь нет, `Current` их не держит.
#[derive(Clone, Copy, Debug)]
struct Node {
    src: u32,
    off: u32,
    len: u32,
    next: Option<NodeId>,
}

/// Перестановка одной ссылки цепочки.
#[derive(Clone, Copy, Debug)]
struct Write {
    link: NodeId,
    to: Option<NodeId>,
}

/// Характеристика операции: как применить, как отменить, на сколько меняется длина.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Surgery {
    apply: Write,
    undo: Write,
    delta: i64,
}

#[derive(Debug)]
pub struct Current {
    nodes: Vec<Node>,
    len: u32,
    original_len: u32,
    base: Option<NodeId>,
}

impl Default for Current {
    fn default() -> Self {
        Self {
            nodes: vec![Node {
                src: 0,
                off: 0,
                len: 0,
                next: None,
            }],
            len: 0,
            original_len: 0,
            base: None,
        }
    }
}

impl Current {
    pub fn from_original(original: &[u8]) -> Self {
        let len = original.len() as u32;
        if len == 0 {
            Self::default()
        } else {
            let nodes = vec![
                Node {
                    src: 0,
                    off: 0,
                    len: 0,
                    next: Some(1),
                },
                Node {
                    src: 0,
                    off: 0,
                    len,
                    next: None,
                },
            ];
            Self {
                nodes,
                len,
                original_len: len,
                base: Some(1),
            }
        }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Нода, которой принадлежит байт `pos`: предыдущая, сама и смещение внутри неё.
    /// `pos` известен заранее в байтах, искать нечего — проход по дескрипторам.
    fn locate(&self, pos: u32) -> (NodeId, Option<NodeId>, u32) {
        debug_assert!(pos <= self.len);
        let mut pre: NodeId = 0;
        let mut cur = self.nodes[0].next;
        let mut offset: u32 = 0;
        while let Some(id) = cur {
            let node = unsafe { self.nodes.get_unchecked(id as usize) };
            if pos.wrapping_sub(offset) < node.len {
                return (pre, Some(id), pos - offset);
            }
            offset += node.len;
            pre = id;
            cur = node.next;
        }
        debug_assert_eq!(pos, self.len, "locate: позиция за концом документа");
        (pre, None, 0)
    }

    pub fn segments<'a>(&'a self, arena: &'a AddArena) -> impl Iterator<Item = Segment> + 'a {
        let mut cur = self.nodes[0].next;
        std::iter::from_fn(move || {
            let id = cur?;
            let node = &self.nodes[id as usize];
            cur = node.next;
            if node.src == 0 {
                Some(Segment::new(Source::Original, node.off, node.len))
            } else {
                let add = node.src - 1;
                let start = arena
                    .start(add)
                    .expect("сегмент ссылается на несуществующую запись arena");
                Some(Segment::new(Source::Add(add), node.off - start, node.len))
            }
        })
    }

    pub fn read(&self, original: &[u8], arena: &AddArena) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len as usize);
        self.read_into(&mut out, original, arena);
        out
    }

    pub fn read_into(&self, out: &mut Vec<u8>, original: &[u8], arena: &AddArena) {
        let bases = [original.as_ptr(), arena.as_ptr()];
        let mut cur = self.nodes[0].next;
        while let Some(id) = cur {
            let node = &self.nodes[id as usize];
            if let Some(nid) = node.next {
                prefetch(self.nodes.as_ptr().wrapping_add(nid as usize).cast());
            }
            let base = unsafe { *bases.get_unchecked((node.src != 0) as usize) };
            let bytes = unsafe { std::slice::from_raw_parts(base.add(node.off as usize), node.len as usize) };
            out.extend_from_slice(bytes);
            cur = node.next;
        }
    }

    /// Инвариант для чтения через сырые указатели: диапазон ноды внутри источника.
    /// В release не вызывается — в горячем цикле чтения проверки быть не должно.
    #[cfg(debug_assertions)]
    fn debug_check_node(&self, node: &Node, original: &[u8], arena: &AddArena) {
        debug_assert!(node.len > 0, "нулевая нода: {:?}", node);
        let (lo, hi) = if node.src == 0 {
            (0u32, original.len() as u32)
        } else {
            let add = node.src - 1;
            let (start, len) = arena
                .range(add)
                .expect("сегмент ссылается на несуществующую запись arena");
            (start, start + len)
        };
        debug_assert!(
            node.off >= lo && node.off + node.len <= hi,
            "нода выходит за границы источника: {:?} при {}", node, hi
        );
    }

    /// Вставка в позицию `pos`. Позиция и длина известны, узел делится на месте курсора.
    pub(crate) fn apply_insert(
        &mut self,
        pos: u32,
        add: AddId,
        off: u32,
        len: u32,
    ) -> Surgery {
        debug_assert!(len > 0);
        let (pre, node, rel) = self.locate(pos);
        let src = add + 1;
        let (apply_to, undo_to) = match node {
            Some(n) if rel == 0 => {
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: Some(n),
                });
                (Some(i), Some(n))
            }
            Some(n) => {
                let old = self.nodes[n as usize];
                let s = self.push(Node {
                    src: old.src,
                    off: old.off + rel,
                    len: old.len - rel,
                    next: old.next,
                });
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: Some(s),
                });
                let p = self.push(Node {
                    src: old.src,
                    off: old.off,
                    len: rel,
                    next: Some(i),
                });
                (Some(p), Some(n))
            }
            None => {
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: None,
                });
                (Some(i), None)
            }
        };
        let apply = Write {
            link: pre,
            to: apply_to,
        };
        self.write(apply);
        self.shift_len(len as i64);
        Surgery {
            apply,
            undo: Write {
                link: pre,
                to: undo_to,
            },
            delta: len as i64,
        }
    }

    /// Удаление диапазона `[pos, pos + len)`.
    pub(crate) fn apply_delete(&mut self, pos: u32, len: u32) -> Surgery {
        debug_assert!(len > 0);
        let end = pos
            .checked_add(len)
            .expect("переполнение pos + len");
        debug_assert!(end <= self.len);
        let (start_pre, start_node, start_rel) = self.locate(pos);
        let (_, end_node, end_rel) = self.locate(end);

        let after = match end_node {
            Some(n) if end_rel == 0 => Some(n),
            Some(n) => {
                let old = self.nodes[n as usize];
                Some(self.push(Node {
                    src: old.src,
                    off: old.off + end_rel,
                    len: old.len - end_rel,
                    next: old.next,
                }))
            }
            None => None,
        };
        let (apply_to, undo_to) = match start_node {
            Some(n) if start_rel > 0 => {
                let old = self.nodes[n as usize];
                let k = self.push(Node {
                    src: old.src,
                    off: old.off,
                    len: start_rel,
                    next: after,
                });
                (Some(k), Some(n))
            }
            Some(n) => (after, Some(n)),
            None => (after, None),
        };
        let apply = Write {
            link: start_pre,
            to: apply_to,
        };
        self.write(apply);
        self.shift_len(-(len as i64));
        Surgery {
            apply,
            undo: Write {
                link: start_pre,
                to: undo_to,
            },
            delta: -(len as i64),
        }
    }

    pub(crate) fn apply_surgery(&mut self, surgery: Surgery) {
        self.write(surgery.apply);
        self.shift_len(surgery.delta);
    }

    pub(crate) fn undo_surgery(&mut self, surgery: Surgery) {
        self.write(surgery.undo);
        self.shift_len(-surgery.delta);
    }

    pub(crate) fn rebuild(&mut self, active: impl Iterator<Item = Surgery>) {
        self.nodes[0].next = self.base;
        self.len = self.original_len;
        for surgery in active {
            self.apply_surgery(surgery);
        }
    }

    #[cfg(test)]
    pub(crate) fn node_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    fn push(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len() as NodeId;
        debug_assert!(id < u32::MAX, "переполнение NodeId");
        self.nodes.push(node);
        id
    }

    fn shift_len(&mut self, delta: i64) {
        let next = self.len as i64 + delta;
        debug_assert!(next >= 0, "длина документа стала отрицательной");
        debug_assert!(next <= u32::MAX as i64, "документ длиннее 4 ГиБ");
        self.len = next as u32;
    }

    fn write(&mut self, write: Write) {
        self.nodes[write.link as usize].next = write.to;
    }
}

#[inline(always)]
fn prefetch(ptr: *const u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_prefetch(ptr as *const i8, core::arch::x86_64::_MM_HINT_T0);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = ptr;
}
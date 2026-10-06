use crate::arena::{AddArena, AddId};
use crate::segment::{Segment, Source};

pub(crate) type NodeId = usize;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Link {
    Head,
    Node(NodeId),
}

#[derive(Clone, Copy, Debug)]
struct Write {
    link: Link,
    to: Option<NodeId>,
}

#[derive(Clone, Copy, Debug)]
struct Split {
    node: NodeId,
    tail: NodeId,
    rel: usize,
    full: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Surgery {
    apply: [Write; 2],
    apply_n: u8,
    undo: [Write; 2],
    undo_n: u8,
    splits: [Option<Split>; 2],
    delta: i64,
}

#[derive(Debug)]
struct Node {
    seg: Segment,
    next: Option<NodeId>,
}

struct Boundary {
    pre: Link,
    node: Option<NodeId>,
    split: Option<Split>,
}

const FILLER: Write = Write {
    link: Link::Head,
    to: None,
};

#[derive(Debug, Default)]
pub struct Current {
    nodes: Vec<Node>,
    head: Option<NodeId>,
    base: Option<NodeId>,
    original_len: usize,
    len: usize,
}

impl Current {
    pub fn from_original(original: &[u8]) -> Self {
        if original.is_empty() {
            Self::default()
        } else {
            let node = Node {
                seg: Segment::new(Source::Original, 0, original.len()),
                next: None,
            };
            Self {
                nodes: vec![node],
                head: Some(0),
                base: Some(0),
                original_len: original.len(),
                len: original.len(),
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        let mut cur = self.head;
        std::iter::from_fn(move || {
            let id = cur?;
            let node = &self.nodes[id];
            cur = node.next;
            Some(node.seg)
        })
    }

    pub fn read(&self, original: &[u8], arena: &AddArena) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len);
        self.read_into(&mut out, original, arena);
        out
    }

    pub fn read_into(&self, out: &mut Vec<u8>, original: &[u8], arena: &AddArena) {
        let mut cur = self.head;
        while let Some(id) = cur {
            let node = &self.nodes[id];
            if let Some(nid) = node.next {
                prefetch(self.nodes.as_ptr().wrapping_add(nid).cast());
            }
            out.extend_from_slice(segment_bytes(node, original, arena));
            cur = node.next;
        }
    }

    pub(crate) fn apply_insert(&mut self, pos: usize, add: AddId, data_len: usize) -> Surgery {
        debug_assert!(pos <= self.len);
        debug_assert!(data_len > 0);
        let boundary = self.resolve(pos);
        let undo_to = match boundary.split {
            Some(split) => self.nodes[split.tail].next,
            None => boundary.node,
        };
        let node = self.nodes.len();
        self.nodes.push(Node {
            seg: Segment::new(Source::Add(add), 0, data_len),
            next: boundary.node,
        });
        let apply = [
            Write {
                link: boundary.pre,
                to: Some(node),
            },
            Write {
                link: Link::Node(node),
                to: boundary.node,
            },
        ];
        self.write(apply[0]);
        self.write(apply[1]);
        self.len += data_len;
        Surgery {
            apply,
            apply_n: 2,
            undo: [Write {
                link: boundary.pre,
                to: undo_to,
            }, FILLER],
            undo_n: 1,
            splits: [boundary.split, None],
            delta: data_len as i64,
        }
    }

    pub(crate) fn apply_delete(&mut self, pos: usize, del_len: usize) -> Surgery {
        debug_assert!(
            pos.checked_add(del_len)
                .is_some_and(|end| end <= self.len)
        );
        debug_assert!(del_len > 0);
        let start = self.resolve(pos);
        let first = start.node.expect("удаление начинается внутри документа");
        let end = self.resolve(pos + del_len);

        let apply = [
            Write {
                link: end.pre,
                to: end.node,
            },
            Write {
                link: start.pre,
                to: end.node,
            },
        ];

        let mut undo = [FILLER; 2];
        let mut undo_n: u8 = 0;
        let mut end_absorb: Option<(NodeId, Option<NodeId>)> = None;
        if let Some(split) = end.split {
            let to = self.nodes[split.tail].next;
            undo[undo_n as usize] = Write {
                link: Link::Node(split.node),
                to,
            };
            undo_n += 1;
            end_absorb = Some((split.node, to));
        }
        match start.split {
            Some(_) => {
                let to = match end_absorb {
                    Some((node, to)) if node == first => to,
                    _ => self.nodes[first].next,
                };
                undo[undo_n as usize] = Write {
                    link: start.pre,
                    to,
                };
                undo_n += 1;
            }
            None => {
                undo[undo_n as usize] = Write {
                    link: start.pre,
                    to: Some(first),
                };
                undo_n += 1;
            }
        }

        self.write(apply[0]);
        self.write(apply[1]);
        self.len -= del_len;
        Surgery {
            apply,
            apply_n: 2,
            undo,
            undo_n,
            splits: [start.split, end.split],
            delta: -(del_len as i64),
        }
    }

    pub(crate) fn apply_surgery(&mut self, surgery: Surgery) {
        for split in surgery.splits.iter().flatten() {
            self.nodes[split.node].seg.len = split.rel;
        }
        for &write in surgery.apply.iter().take(surgery.apply_n as usize) {
            self.write(write);
        }
        self.shift_len(surgery.delta);
    }

    pub(crate) fn undo_surgery(&mut self, surgery: Surgery) {
        for &write in surgery.undo.iter().take(surgery.undo_n as usize) {
            self.write(write);
        }
        for split in surgery.splits.iter().flatten() {
            self.nodes[split.node].seg.len = split.full;
        }
        self.shift_len(-surgery.delta);
    }

    pub(crate) fn rebuild(&mut self, active: impl Iterator<Item = Surgery>) {
        self.head = self.base;
        self.len = self.original_len;
        for surgery in active {
            self.apply_surgery(surgery);
        }
    }

    #[cfg(test)]
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn shift_len(&mut self, delta: i64) {
        let next = self.len as i64 + delta;
        debug_assert!(next >= 0, "длина документа стала отрицательной");
        self.len = next as usize;
    }

    fn write(&mut self, write: Write) {
        match write.link {
            Link::Head => self.head = write.to,
            Link::Node(id) => self.nodes[id].next = write.to,
        }
    }

    fn resolve(&mut self, pos: usize) -> Boundary {
        let mut cur = self.head;
        let mut prev: Option<NodeId> = None;
        let mut offset = 0usize;
        while let Some(id) = cur {
            let seg = self.nodes[id].seg;
            if offset + seg.len > pos {
                let rel = pos - offset;
                if rel == 0 {
                    return Boundary {
                        pre: prev.map_or(Link::Head, Link::Node),
                        node: Some(id),
                        split: None,
                    };
                }
                let tail = Node {
                    seg: Segment::new(seg.source, seg.offset + rel, seg.len - rel),
                    next: self.nodes[id].next,
                };
                let tail_id = self.nodes.len();
                self.nodes.push(tail);
                self.nodes[id].seg.len = rel;
                self.nodes[id].next = Some(tail_id);
                return Boundary {
                    pre: Link::Node(id),
                    node: Some(tail_id),
                    split: Some(Split {
                        node: id,
                        tail: tail_id,
                        rel,
                        full: seg.len,
                    }),
                };
            }
            offset += seg.len;
            prev = Some(id);
            cur = self.nodes[id].next;
        }
        debug_assert_eq!(pos, self.len, "resolve вышел за конец документа");
        Boundary {
            pre: prev.map_or(Link::Head, Link::Node),
            node: None,
            split: None,
        }
    }
}

#[inline(always)]
fn segment_bytes<'a>(node: &Node, original: &'a [u8], arena: &'a AddArena) -> &'a [u8] {
    match node.seg.source {
        Source::Original => &original[node.seg.offset..node.seg.end()],
        Source::Add(add) => &arena[add][node.seg.offset..node.seg.end()],
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

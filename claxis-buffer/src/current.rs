use crate::arena::{AddArena, AddId};
use crate::segment::{Segment, Source};

pub(crate) type NodeId = u32;

#[derive(Clone, Copy, Debug)]
struct Node {
    src: u32,
    off: usize,
    len: usize,
    next: Option<NodeId>,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    link: NodeId,
    to: Option<NodeId>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Surgery {
    apply: Write,
    undo: Write,
    delta: i64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    pub(crate) pre: NodeId,
    pub(crate) node: Option<NodeId>,
    pub(crate) rel: usize,
    pub(crate) pos: usize,
}

#[derive(Debug)]
pub struct Current {
    nodes: Vec<Node>,
    len: usize,
    original_len: usize,
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
        if original.is_empty() {
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
                    len: original.len(),
                    next: None,
                },
            ];
            Self {
                nodes,
                len: original.len(),
                original_len: original.len(),
                base: Some(1),
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
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
                let add = node.src as usize - 1;
                let start = arena
                    .start(add)
                    .expect("сегмент ссылается на несуществующую запись arena");
                Some(Segment::new(Source::Add(add), node.off - start, node.len))
            }
        })
    }

    pub fn read(&self, original: &[u8], arena: &AddArena) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len);
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
            let bytes = unsafe { std::slice::from_raw_parts(base.add(node.off), node.len) };
            out.extend_from_slice(bytes);
            cur = node.next;
        }
    }

    pub(crate) fn cursor_at(&self, pos: usize) -> Cursor {
        debug_assert!(pos <= self.len);
        let mut pre: NodeId = 0;
        let mut cur = self.nodes[0].next;
        let mut offset = 0usize;
        while let Some(id) = cur {
            let node = &self.nodes[id as usize];
            if offset + node.len > pos {
                return Cursor {
                    pre,
                    node: Some(id),
                    rel: pos - offset,
                    pos,
                };
            }
            offset += node.len;
            pre = id;
            cur = node.next;
        }
        debug_assert_eq!(pos, self.len, "cursor_at вышел за конец документа");
        Cursor {
            pre,
            node: None,
            rel: 0,
            pos,
        }
    }

    pub(crate) fn apply_insert(
        &mut self,
        cursor: Cursor,
        add: AddId,
        off: usize,
        data_len: usize,
    ) -> Surgery {
        debug_assert!(data_len > 0);
        debug_assert!(add < u32::MAX as usize);
        let src = add as u32 + 1;
        let (apply_to, undo_to) = match cursor.node {
            Some(n) if cursor.rel == 0 => {
                let i = self.push(Node {
                    src,
                    off,
                    len: data_len,
                    next: Some(n),
                });
                (Some(i), Some(n))
            }
            Some(n) => {
                let old = self.nodes[n as usize];
                let s = self.push(Node {
                    src: old.src,
                    off: old.off + cursor.rel,
                    len: old.len - cursor.rel,
                    next: old.next,
                });
                let i = self.push(Node {
                    src,
                    off,
                    len: data_len,
                    next: Some(s),
                });
                let p = self.push(Node {
                    src: old.src,
                    off: old.off,
                    len: cursor.rel,
                    next: Some(i),
                });
                (Some(p), Some(n))
            }
            None => {
                let i = self.push(Node {
                    src,
                    off,
                    len: data_len,
                    next: None,
                });
                (Some(i), None)
            }
        };
        let apply = Write {
            link: cursor.pre,
            to: apply_to,
        };
        self.write(apply);
        self.len += data_len;
        Surgery {
            apply,
            undo: Write {
                link: cursor.pre,
                to: undo_to,
            },
            delta: data_len as i64,
        }
    }

    pub(crate) fn apply_delete(&mut self, start: Cursor, end: Cursor) -> Surgery {
        debug_assert!(start.pos < end.pos);
        debug_assert!(end.pos <= self.len);
        let after = match end.node {
            Some(n) if end.rel == 0 => Some(n),
            Some(n) => {
                let old = self.nodes[n as usize];
                let t = self.push(Node {
                    src: old.src,
                    off: old.off + end.rel,
                    len: old.len - end.rel,
                    next: old.next,
                });
                Some(t)
            }
            None => None,
        };
        let (apply_to, undo_to) = match start.node {
            Some(n) if start.rel > 0 => {
                let old = self.nodes[n as usize];
                let k = self.push(Node {
                    src: old.src,
                    off: old.off,
                    len: start.rel,
                    next: after,
                });
                (Some(k), Some(n))
            }
            Some(n) => (after, Some(n)),
            None => (after, None),
        };
        let apply = Write {
            link: start.pre,
            to: apply_to,
        };
        self.write(apply);
        let delta = end.pos - start.pos;
        self.len -= delta;
        Surgery {
            apply,
            undo: Write {
                link: start.pre,
                to: undo_to,
            },
            delta: -(delta as i64),
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
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn push(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(node);
        id
    }

    fn shift_len(&mut self, delta: i64) {
        let next = self.len as i64 + delta;
        debug_assert!(next >= 0, "длина документа стала отрицательной");
        self.len = next as usize;
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

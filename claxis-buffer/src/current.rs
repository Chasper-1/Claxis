use crate::arena::{AddArena, AddId};
use crate::segment::{Segment, Source};

pub(crate) type NodeId = u32;

/// Узел — только ссылки: источник, смещение, длина, соседи.
/// Записей истории здесь нет, `Current` их не держит.
#[derive(Clone, Copy, Debug)]
struct Node {
    src: u32,
    off: u32,
    len: u32,
    next: Option<NodeId>,
    prev: NodeId,
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
    /// Отслеженное положение курсора: известная позиция, предыдущая нода,
    /// сама нода и смещение в ней. Отсюда считается следующая позиция —
    /// прибавлением длин, а не обходом от головы.
    cursor_pos: u32,
    cursor_pre: NodeId,
    cursor_node: Option<NodeId>,
    cursor_rel: u32,
}

impl Default for Current {
    fn default() -> Self {
        Self {
            nodes: vec![Node {
                src: 0,
                off: 0,
                len: 0,
                next: None,
                prev: 0,
            }],
            len: 0,
            original_len: 0,
            base: None,
            cursor_pos: 0,
            cursor_pre: 0,
            cursor_node: None,
            cursor_rel: 0,
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
                    prev: 0,
                },
                Node {
                    src: 0,
                    off: 0,
                    len,
                    next: None,
                    prev: 0,
                },
            ];
            Self {
                nodes,
                len,
                original_len: len,
                base: Some(1),
                cursor_pos: 0,
                cursor_pre: 0,
                cursor_node: Some(1),
                cursor_rel: 0,
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
    ///
    /// Считается от отслеженного положения курсора: разница позиций — это
    /// сумма длин нод на пути, то есть прибавление по ходу, а не обход от головы.
    fn locate(&mut self, pos: u32) -> (NodeId, Option<NodeId>, u32) {
        debug_assert!(pos <= self.len);
        let mut pre = self.cursor_pre;
        let mut node = self.cursor_node;
        let mut rel = self.cursor_rel;
        let mut offset = self.cursor_pos;

        if pos >= offset {
            // Разница позиций — сумма длин: идём вперёд, прибавляя длины.
            while offset < pos {
                let id = match node {
                    Some(id) => id,
                    None => {
                        debug_assert_eq!(pos, self.len, "locate: позиция за концом документа");
                        break;
                    }
                };
                let n = self.nodes[id as usize];
                let room = n.len - rel;
                let step = pos - offset;
                if step < room {
                    // Позиция внутри этой ноды — дальше идти не надо.
                    rel += step;
                    break;
                }
                if let Some(next) = n.next {
                    prefetch(self.nodes.as_ptr().wrapping_add(next as usize).cast());
                }
                offset += room;
                pre = id;
                node = n.next;
                rel = 0;
            }
        } else {
            // Назад: та же арифметика, но вычитанием.
            while offset > pos {
                let back = offset - pos;
                if back <= rel {
                    rel -= back;
                    break;
                }
                offset -= rel;
                let id = pre;
                if id == 0 {
                    debug_assert_eq!(pos, 0, "locate: позиция до начала документа");
                    node = None;
                    rel = 0;
                    pre = 0;
                    break;
                }
                let n = self.nodes[id as usize];
                if n.prev != 0 {
                    prefetch(self.nodes.as_ptr().wrapping_add(n.prev as usize).cast());
                }
                pre = n.prev;
                node = Some(id);
                rel = n.len;
            }
        }

        let found = (pre, node, rel);
        self.set_cursor(pos, found);
        found
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
            #[cfg(debug_assertions)]
            self.debug_check_node(node, original, arena);
            let base = unsafe { *bases.get_unchecked((node.src != 0) as usize) };
            let bytes = unsafe {
                std::slice::from_raw_parts(base.add(node.off as usize), node.len as usize)
            };
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
            "нода выходит за границы источника: {:?} при {}",
            node,
            hi
        );
    }

    /// Вставка в позицию `pos`. Позиция и длина известны, узел делится на месте курсора.
    pub(crate) fn apply_insert(&mut self, pos: u32, add: AddId, off: u32, len: u32) -> Surgery {
        debug_assert!(len > 0);
        let (pre, node, rel) = self.locate(pos);
        // Курсор может стоять ровно на конце ноды — делить её нельзя,
        // пустой хвост. Переносим вставку за эту ноду.
        let (pre, node, rel) = match (pre, node, rel) {
            (_, Some(n), r) if r >= self.nodes[n as usize].len => {
                (n, self.nodes[n as usize].next, 0)
            }
            other => other,
        };
        let src = add + 1;
        let (apply_to, undo_to, cursor) = match node {
            Some(n) if rel == 0 => {
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: Some(n),
                    prev: pre,
                });
                self.nodes[n as usize].prev = i;
                (Some(i), Some(n), (pre, Some(i), len))
            }
            Some(n) => {
                let old = self.nodes[n as usize];
                let s = self.push(Node {
                    src: old.src,
                    off: old.off + rel,
                    len: old.len - rel,
                    next: old.next,
                    prev: 0,
                });
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: Some(s),
                    prev: 0,
                });
                let p = self.push(Node {
                    src: old.src,
                    off: old.off,
                    len: rel,
                    next: Some(i),
                    prev: pre,
                });
                self.nodes[s as usize].prev = i;
                self.nodes[i as usize].prev = p;
                if let Some(next) = old.next {
                    self.nodes[next as usize].prev = s;
                }
                (Some(p), Some(n), (p, Some(i), len))
            }
            None => {
                let i = self.push(Node {
                    src,
                    off,
                    len,
                    next: None,
                    prev: pre,
                });
                (Some(i), None, (i, None, 0))
            }
        };
        let apply = Write {
            link: pre,
            to: apply_to,
        };
        self.write(apply);
        self.shift_len(len as i64);
        self.set_cursor(pos + len, cursor);
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
        let end = pos.checked_add(len).expect("переполнение pos + len");
        debug_assert!(end <= self.len);
        let (start_pre, start_node, start_rel) = self.locate(pos);
        let (_, end_node, end_rel) = self.locate(end);

        let after = match end_node {
            Some(n) if end_rel == 0 => Some(n),
            Some(n) => {
                let old = self.nodes[n as usize];
                let t = self.push(Node {
                    src: old.src,
                    off: old.off + end_rel,
                    len: old.len - end_rel,
                    next: old.next,
                    prev: 0,
                });
                if let Some(next) = old.next {
                    self.nodes[next as usize].prev = t;
                }
                Some(t)
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
                    prev: start_pre,
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
        // Узел сразу за оставшейся частью теперь идёт после неё.
        if let Some(a) = apply_to {
            if let Some(succ) = self.nodes[a as usize].next {
                self.nodes[succ as usize].prev = a;
            }
        }
        self.shift_len(-(len as i64));
        self.set_cursor(pos, (start_pre, apply_to, start_rel));
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
        self.reset_cursor();
    }

    pub(crate) fn undo_surgery(&mut self, surgery: Surgery) {
        self.write(surgery.undo);
        self.shift_len(-surgery.delta);
        self.reset_cursor();
    }

    pub(crate) fn rebuild(&mut self, active: impl Iterator<Item = Surgery>) {
        self.nodes[0].next = self.base;
        if let Some(base) = self.base {
            self.nodes[base as usize].prev = 0;
        }
        self.len = self.original_len;
        for surgery in active {
            self.write(surgery.apply);
            self.shift_len(surgery.delta);
        }
        self.reset_cursor();
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
        if let Some(to) = write.to {
            self.nodes[to as usize].prev = write.link;
        }
    }

    /// Запомнить, где теперь стоит курсор.
    fn set_cursor(&mut self, pos: u32, cursor: (NodeId, Option<NodeId>, u32)) {
        self.cursor_pos = pos;
        self.cursor_pre = cursor.0;
        self.cursor_node = cursor.1;
        self.cursor_rel = cursor.2;
    }

    /// Сброс отслеживания: цепочка перестроена, прежняя нода могла выпасть.
    /// Отсчёт от головы цепочки, а не от `base`: при пустом оригинале `base` нет.
    fn reset_cursor(&mut self) {
        let head = self.nodes[0].next;
        self.set_cursor(0, (0, head, 0));
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

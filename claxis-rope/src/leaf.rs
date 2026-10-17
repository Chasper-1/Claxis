//! Лист rope: диапазон общего буфера.

/// Сколько чанков тянет лист. Больше двух — делится.
pub const MAX_CHUNKS: usize = 2;

/// Лист: диапазон общего буфера от `start` до `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub(crate) start: u32,
    pub(crate) end: u32,
}

impl Leaf {
    /// Лист, которым заполняют новый: с нуля до нуля.
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    /// Лист над диапазоном `start..end`.
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Начало диапазона.
    pub fn start(&self) -> u32 {
        self.start
    }

    /// Конец диапазона.
    pub fn end(&self) -> u32 {
        self.end
    }

    /// Длина листа в байтах.
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Лист пуст, если в нём нет ни байта.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Сколько чанков тянет лист: `1`, если лист не пуст, иначе `0`.
    pub fn chunks(&self, chunk: usize) -> usize {
        let Some(last) = (self.end as usize).checked_sub(1) else {
            return 0;
        };
        if self.is_empty() {
            return 0;
        }
        let first = self.start as usize / chunk;
        last / chunk - first + 1
    }

    /// Делит лист по границе чанка: левая половина тянет не больше двух чанков,
    /// правая — то, что осталось, и может потребовать ещё деления.
    pub fn split(&self, chunk: usize) -> (Self, Self) {
        let first = self.start as usize / chunk;
        let middle = ((first + MAX_CHUNKS) * chunk) as u32;
        (Self::new(self.start, middle), Self::new(middle, self.end))
    }

    /// Съедает соседний лист справа. Сосед не вплотную или вместе больше двух
    /// чанков — лист остаётся как был.
    pub fn absorb(&mut self, other: &Self, chunk: usize) -> bool {
        if other.is_empty() || self.end != other.start {
            return false;
        }
        let joined = Self::new(self.start, other.end);
        if joined.chunks(chunk) > MAX_CHUNKS {
            return false;
        }
        *self = joined;
        true
    }

    /// Двигает диапазон на `delta` байт: вставка толкает вправо, вырезание — влево.
    /// Лист, начинающийся раньше `at`, не двигается: правка не в нём.
    pub fn shift_by(&mut self, at: u32, delta: i64) {
        if self.start < at {
            return;
        }
        self.start = (self.start as i64 + delta) as u32;
        self.end = (self.end as i64 + delta) as u32;
    }

    /// Вставляет `len` байт в позицию `at` внутри листа или на его конце.
    pub fn extend(&mut self, at: u32, len: u32) {
        if at > self.end {
            return;
        }
        self.end += len;
    }

    /// Вырезает `len` байт начиная с позиции `at`. За листом ничего не вырезается.
    pub fn cut(&mut self, at: u32, len: u32) {
        if self.end <= at {
            return;
        }
        let from = at.max(self.start);
        self.end -= len.min(self.end - from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_counted_by_boundaries() {
        let leaf = Leaf::new(0, 10);
        assert_eq!(leaf.chunks(512), 1);
        assert_eq!(leaf.chunks(8), 2);
        assert_eq!(leaf.chunks(4), 3);
        assert_eq!(Leaf::new(3, 3).chunks(4), 0);
    }

    #[test]
    fn a_leaf_over_two_chunks_splits_on_a_chunk_border() {
        let leaf = Leaf::new(0, 5 * 512);
        assert_eq!(leaf.chunks(512), 5);
        let (left, right) = leaf.split(512);
        assert_eq!(left, Leaf::new(0, 2 * 512));
        assert_eq!(right, Leaf::new(2 * 512, 5 * 512));
        assert_eq!(left.chunks(512), MAX_CHUNKS);
        // Правая половина ещё широка двух чанков: её делят ещё раз.
        let (right, tail) = right.split(512);
        assert_eq!(right, Leaf::new(2 * 512, 4 * 512));
        assert_eq!(tail, Leaf::new(4 * 512, 5 * 512));
        assert!(right.chunks(512) <= MAX_CHUNKS);
        assert!(tail.chunks(512) <= MAX_CHUNKS);
    }

    #[test]
    fn neighbours_merge_while_they_fit_two_chunks() {
        let mut leaf = Leaf::new(0, 512);
        assert!(leaf.absorb(&Leaf::new(512, 1024), 512));
        assert_eq!(leaf, Leaf::new(0, 1024));
        assert!(!leaf.absorb(&Leaf::new(1024, 1536), 512));
        assert_eq!(leaf, Leaf::new(0, 1024));
    }

    #[test]
    fn a_gap_keeps_leaves_apart() {
        let mut leaf = Leaf::new(0, 512);
        assert!(!leaf.absorb(&Leaf::new(513, 600), 512));
        assert_eq!(leaf, Leaf::new(0, 512));
    }

    #[test]
    fn a_cut_takes_what_is_left_after_the_start() {
        let mut leaf = Leaf::new(10, 20);
        leaf.cut(15, 100);
        assert_eq!(leaf.end(), 15);
        leaf.cut(30, 5);
        assert_eq!(leaf.end(), 15);
    }
}

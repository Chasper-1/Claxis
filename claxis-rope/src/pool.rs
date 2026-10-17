//! Пул узлов и листьев: освобождённое переиспользуется, а не отдаётся аллокатору.

/// Пул значений одного вида.
#[derive(Debug, Default)]
pub struct Pool<T> {
    free: Vec<T>,
    len: usize,
}

impl<T> Pool<T> {
    /// Пустой пул.
    pub fn new() -> Self {
        Self {
            free: Vec::new(),
            len: 0,
        }
    }

    /// Значение из пула, если оно есть, иначе `make()`.
    pub fn take(&mut self, make: impl FnOnce() -> T) -> T {
        let value = self.free.pop().unwrap_or_else(make);
        self.len += 1;
        value
    }

    /// Возвращает значение в пул.
    pub fn give(&mut self, value: T) {
        self.len -= 1;
        self.free.push(value);
    }

    /// Сколько значений пул держит для переиспользования.
    pub fn free(&self) -> usize {
        self.free.len()
    }

    /// Сколько значений пул выдал и ещё не забрал.
    pub fn live(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_value_comes_back_from_the_pool() {
        let mut pool = Pool::new();
        let value = pool.take(|| 7);
        assert_eq!(value, 7);
        assert_eq!(pool.live(), 1);
        pool.give(7);
        assert_eq!(pool.live(), 0);
        assert_eq!(pool.take(|| 0), 7);
    }

    #[test]
    fn an_empty_pool_builds_its_own_value() {
        let mut pool = Pool::new();
        assert_eq!(pool.take(|| 7), 7);
        assert_eq!(pool.free(), 0);
    }
}

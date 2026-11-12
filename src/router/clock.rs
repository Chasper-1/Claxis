//! Источник времени.
//!
//! Отдельный тип, а не `Instant::now()` в коде: иначе проверить, что сработало
//! правильно, можно было бы только ожиданием вживую.

use std::time::Duration;

/// Источник времени.
pub trait Clock: std::fmt::Debug {
    /// Сколько прошло с начала работы процесса.
    fn elapsed(&self) -> Duration;
}

/// Часы, которые двигает тест: ждать настоящие пять минут нельзя.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub Duration);

impl FixedClock {
    /// Часы, прошедшие указанное время от старта процесса.
    pub fn after(elapsed: Duration) -> Self {
        Self(elapsed)
    }
}

impl Clock for FixedClock {
    fn elapsed(&self) -> Duration {
        self.0
    }
}

/// Настоящие часы.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FixedClock, SystemClock};
    use std::time::Duration;

    #[test]
    fn a_fixed_clock_stands_still() {
        let clock = FixedClock::after(Duration::from_secs(600));
        assert_eq!(clock.elapsed(), Duration::from_secs(600));
        // Время не идёт само: двигает его только тот, кто двигает.
        assert_eq!(clock.elapsed(), Duration::from_secs(600));
    }

    #[test]
    fn the_system_clock_only_goes_forward() {
        let clock = SystemClock;
        let first = clock.elapsed();
        let second = clock.elapsed();
        assert!(second >= first, "часы пошли назад: {first:?} → {second:?}");
    }
}

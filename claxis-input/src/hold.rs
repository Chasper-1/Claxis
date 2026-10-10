//! Нажатие и удержание: одно отличается от другого по времени.
//!
//! Редактор различает нажатие и отпускание, потому что на них построена
//! работа с режимами: удержание открывает режим, и всё, что набирается в
//! нём, применяется при отпускании. Без события отпускания это невозможно,
//! поэтому терминал обязан поддерживать kitty keyboard protocol.
//!
//! Решение простое: у каждой клавиши со двумя значениями есть своё окно в
//! миллисекундах. Успел отпустить внутри окна — обычное действие. Не успел —
//! удержание.
//!
//! Окно действует **только** на клавиши, у которых два значения. На все клавиши
//! подряд оно распространяться не может: тогда каждый введённый символ
//! приходил бы с задержкой в окно и печатать было бы невозможно.

use std::collections::BTreeMap;
use std::time::Duration;

/// Имя клавиши в настройках и её окно удержания.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HoldKey {
    /// Имя клавиши: `alt`, `ctrl`, `space`.
    pub name: String,
    /// Окно, за которое нажатие считается обычным.
    pub window: Duration,
}

impl HoldKey {
    /// Окно в миллисекундах — так оно записано в конфиге.
    pub fn window_ms(&self) -> u128 {
        self.window.as_millis()
    }
}

/// Настройки удержания: окно для каждой клавиши.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hold {
    /// Окна по именам клавиш. Порядок не важен.
    keys: BTreeMap<String, Duration>,
}

impl Hold {
    /// Пустые настройки: удержания нет ни у одной клавиши.
    pub fn new() -> Self {
        Self::default()
    }

    /// Настройки по умолчанию.
    pub fn defaults() -> Self {
        Self::from_pairs(
            crate::defaults::DEFAULT_HOLD
                .iter()
                .map(|(k, ms)| (*k, *ms)),
        )
    }

    /// Настройки из пар «клавиша, миллисекунды».
    pub fn from_pairs(pairs: impl IntoIterator<Item = (&'static str, u32)>) -> Self {
        Self {
            keys: pairs
                .into_iter()
                .map(|(name, ms)| (name.to_string(), Duration::from_millis(ms as u64)))
                .collect(),
        }
    }

    /// Добавить или заменить окно клавиши.
    pub fn set(&mut self, name: impl Into<String>, window: Duration) {
        self.keys.insert(name.into(), window);
    }

    /// Окно клавиши, если она настроена.
    pub fn window(&self, name: &str) -> Option<Duration> {
        self.keys.get(name).copied()
    }

    /// Клавиша настроена на два значения?
    pub fn holds(&self, name: &str) -> bool {
        self.keys.contains_key(name)
    }

    /// Все настроенные клавиши.
    pub fn keys(&self) -> impl Iterator<Item = HoldKey> + '_ {
        self.keys.iter().map(|(name, window)| HoldKey {
            name: name.clone(),
            window: *window,
        })
    }

    /// Сколько клавиш настроено.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Ни одной клавиши не настроено.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Что означает нажатие: обычное действие или удержание.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Нажали и отпустили в пределах окна — обычное действие.
    Tap,
    /// Окно истекло, а палец всё ещё на клавише — удержание.
    Held,
}

/// Что решить из нажатия и отпускания.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// Отпускания ещё не было: ждём, пока истечёт окно.
    Waiting,
    /// Отпустились вовремя — обычное действие.
    Tapped,
    /// Окно истекло, отпускания не было — удержание.
    Held,
}

/// Решить, тап это или удержание.
///
/// `held_ms` — сколько прошло с нажатия. Клавиши без настроенного окна
/// всегда считаются обычным нажатием: у них нет второго значения.
pub fn resolve(hold: &Hold, key: &str, held_ms: u64) -> Resolved {
    match hold.window(key) {
        None => Resolved::Tapped,
        Some(window) => {
            if held_ms < window_ms(window) {
                Resolved::Tapped
            } else {
                Resolved::Held
            }
        }
    }
}

/// Окно в целых миллисекундах.
fn window_ms(window: Duration) -> u64 {
    window.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold() -> Hold {
        Hold::from_pairs([("alt", 200), ("ctrl", 200)])
    }

    #[test]
    fn defaults_have_two_keys_of_200() {
        let h = Hold::defaults();
        assert_eq!(h.len(), 2, "по умолчанию две клавиши");
        assert_eq!(h.window("alt"), Some(Duration::from_millis(200)));
        assert_eq!(h.window("ctrl"), Some(Duration::from_millis(200)));
    }

    #[test]
    fn quick_release_is_a_tap() {
        // Нажали и отпустили быстро — обычное действие.
        assert_eq!(resolve(&hold(), "alt", 0), Resolved::Tapped);
        assert_eq!(resolve(&hold(), "alt", 100), Resolved::Tapped);
        assert_eq!(resolve(&hold(), "alt", 199), Resolved::Tapped);
    }

    #[test]
    fn slow_release_is_a_hold() {
        // Окно истекло, палец не отпустил — удержание.
        assert_eq!(resolve(&hold(), "alt", 200), Resolved::Held);
        assert_eq!(resolve(&hold(), "alt", 5000), Resolved::Held);
    }

    #[test]
    fn each_key_has_its_own_window() {
        let mut h = hold();
        h.set("space", Duration::from_millis(50));
        // Быстрые клавиши удержанием не считаются.
        assert_eq!(resolve(&h, "space", 60), Resolved::Held);
        // Долгие по-прежнему ждут своё окно.
        assert_eq!(resolve(&h, "alt", 60), Resolved::Tapped);
    }

    #[test]
    fn plain_key_is_always_a_tap() {
        // У клавиши без окна второго значения нет: она не может удерживаться.
        // Иначе каждый символ ждал бы окончания окна.
        assert_eq!(resolve(&hold(), "a", 10_000), Resolved::Tapped);
        assert!(!hold().holds("a"));
    }

    #[test]
    fn keys_can_be_added_without_limit() {
        let mut h = hold();
        h.set("space", Duration::from_millis(120));
        h.set("tab", Duration::from_millis(300));
        assert_eq!(h.len(), 4);
        assert_eq!(h.window("tab"), Some(Duration::from_millis(300)));
    }

    #[test]
    fn empty_hold_makes_everything_a_tap() {
        // Пока удержание не настроено, редактор работает как обычный.
        let h = Hold::new();
        assert!(h.is_empty());
        assert_eq!(resolve(&h, "alt", 9999), Resolved::Tapped);
    }

    #[test]
    fn window_is_reported_in_milliseconds() {
        let key = HoldKey {
            name: "alt".to_string(),
            window: Duration::from_millis(200),
        };
        assert_eq!(key.window_ms(), 200);
    }
}

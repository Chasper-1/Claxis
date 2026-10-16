//! Таблица привязок по уровням.
//!
//! Алиасы и хоткеи хранятся одинаково: ключ, уровень, команда. Различаются
//! только ключи — символы у алиасов, последовательности позиций у хоткеев.
//! И живут они независимо: одна таблица на другой не влияет.
//!
//! Одна и та же комбинация на двух уровнях — ошибка, а не «победил верхний».
//! Случайно перекрытая привязка должна быть видна сразу, при загрузке
//! конфига, а не всплывать в момент нажатия.

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

use crate::error::Error;
use crate::level::Level;
use crate::name::CommandName;

/// Привязки одного вида по всем уровням.
#[derive(Clone, Debug)]
pub struct Bindings<K> {
    /// Ключ и уровень, по которому ищем, — команды.
    table: HashMap<(K, Level), CommandName>,
}

impl<K> Default for Bindings<K> {
    fn default() -> Self {
        Self {
            table: HashMap::new(),
        }
    }
}

impl<K> Bindings<K>
where
    K: Eq + Hash + Clone + Ord + Display,
{
    /// Новая пустая таблица.
    pub fn new() -> Self {
        Self::default()
    }

    /// Привязать ключ на уровне.
    ///
    /// Тот же ключ на том же уровне перезаписывается: это обычная правка
    /// конфига. Тот же ключ на другом уровне — ошибка.
    pub fn bind(&mut self, key: K, level: Level, command: CommandName) -> Result<(), Error> {
        // Поиск линейный не из-за скорости: уровней мало, а таблица
        // перестраивается только при загрузке конфига.
        for (taken_key, taken_level, _) in self.all() {
            if taken_key == &key && taken_level != &level {
                return Err(Error::BindingConflict {
                    key: key.to_string(),
                    first: taken_level.section(),
                    second: level.section(),
                });
            }
        }
        self.table.insert((key, level), command);
        Ok(())
    }

    /// Команда по ключу, если она есть на этом уровне.
    pub fn get(&self, key: &K, level: &Level) -> Option<&CommandName> {
        self.table.get(&(key.clone(), level.clone()))
    }

    /// Команда по ключу с учётом уровней.
    ///
    /// Уровни проверяются в том порядке, в котором даны: первый подошедший
    /// выигрывает. Редактор передаёт их от `global` к самому узкому.
    pub fn resolve(&self, key: &K, levels: &[Level]) -> Option<&CommandName> {
        levels.iter().find_map(|level| self.get(key, level))
    }

    /// Убрать привязку. Не было — ничего не делает.
    pub fn unbind(&mut self, key: &K, level: &Level) -> Option<CommandName> {
        self.table.remove(&(key.clone(), level.clone()))
    }

    /// Все привязки на уровне: ключ и команда.
    pub fn on(&self, level: &Level) -> Vec<(&K, &CommandName)> {
        let mut pairs: Vec<_> = self
            .table
            .iter()
            .filter(|((_, l), _)| l == level)
            .map(|((k, _), c)| (k, c))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        pairs
    }

    /// Все привязки целиком: ключ, уровень, команда.
    pub fn all(&self) -> Vec<(&K, &Level, &CommandName)> {
        let mut triples: Vec<_> = self.table.iter().map(|((k, l), c)| (k, l, c)).collect();
        triples.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(b.1)));
        triples
    }

    /// Привязок всего.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Есть ли хоть одна привязка.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> CommandName {
        CommandName::parse(text).unwrap()
    }

    type Aliases = Bindings<crate::alias::Alias>;

    fn alias(text: &str) -> crate::alias::Alias {
        crate::alias::Alias::new(text).unwrap()
    }

    #[test]
    fn a_binding_is_found_on_its_own_level() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::line"))
            .unwrap();
        assert_eq!(
            aliases.get(&alias("dd"), &Level::Global),
            Some(&name(":std::edit::delete::line"))
        );
    }

    #[test]
    fn the_same_binding_on_two_levels_is_a_conflict() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::line"))
            .unwrap();
        let error = aliases
            .bind(
                alias("dd"),
                Level::State("edit".into()),
                name(":std::edit::delete::word"),
            )
            .unwrap_err();
        assert!(matches!(error, Error::BindingConflict { .. }));
    }

    #[test]
    fn the_same_binding_on_one_level_is_a_rebind_not_a_conflict() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::line"))
            .unwrap();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::word"))
            .unwrap();
        assert_eq!(
            aliases.get(&alias("dd"), &Level::Global),
            Some(&name(":std::edit::delete::word"))
        );
        assert_eq!(aliases.len(), 1);
    }

    #[test]
    fn a_key_found_on_a_lower_level_still_resolves_through_the_chain() {
        // Выбирать между уровнями не приходится: тот же ключ на двух уровнях
        // не заводится. Цепочка уровней нужна для поиска, а не для выбора.
        let mut aliases = Aliases::new();
        aliases
            .bind(
                alias("h"),
                Level::State("edit".into()),
                name(":std::edit::move::left"),
            )
            .unwrap();
        aliases
            .bind(alias("j"), Level::Global, name(":std::edit::move::down"))
            .unwrap();
        let levels = [Level::Global, Level::State("edit".into())];
        assert_eq!(
            aliases.resolve(&alias("h"), &levels),
            Some(&name(":std::edit::move::left")),
            "верхний уровень молчит, нижний отвечает"
        );
        assert_eq!(
            aliases.resolve(&alias("j"), &levels),
            Some(&name(":std::edit::move::down"))
        );
    }

    #[test]
    fn an_unknown_key_resolves_to_nothing() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::line"))
            .unwrap();
        assert_eq!(aliases.resolve(&alias("xx"), &[Level::Global]), None);
    }

    #[test]
    fn levels_stay_independent_when_the_key_differs() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("j"), Level::Global, name(":std::edit::move::down"))
            .unwrap();
        aliases
            .bind(
                alias("h"),
                Level::State("edit".into()),
                name(":std::edit::move::left"),
            )
            .unwrap();
        let levels = [Level::Global, Level::State("edit".into())];
        assert_eq!(
            aliases.resolve(&alias("h"), &levels),
            Some(&name(":std::edit::move::left"))
        );
        assert_eq!(aliases.len(), 2);
    }

    #[test]
    fn unbinding_removes_exactly_one_level() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("p"), Level::Global, name(":std::clipboard::paste"))
            .unwrap();
        aliases
            .bind(
                alias("V"),
                Level::State("insert".into()),
                name(":std::edit::paste::literal"),
            )
            .unwrap();
        assert_eq!(
            aliases.unbind(&alias("p"), &Level::Global),
            Some(name(":std::clipboard::paste"))
        );
        assert_eq!(aliases.get(&alias("p"), &Level::Global), None);
        assert_eq!(
            aliases.get(&alias("V"), &Level::State("insert".into())),
            Some(&name(":std::edit::paste::literal")),
            "соседний уровень не тронут"
        );
        assert_eq!(
            aliases.unbind(&alias("p"), &Level::Global),
            None,
            "второй раз отвязывать нечего"
        );
    }

    #[test]
    fn bindings_are_listed_level_by_level() {
        let mut aliases = Aliases::new();
        aliases
            .bind(alias("yy"), Level::Global, name(":std::edit::copy::line"))
            .unwrap();
        aliases
            .bind(alias("dd"), Level::Global, name(":std::edit::delete::line"))
            .unwrap();
        let on_global = aliases.on(&Level::Global);
        assert_eq!(on_global.len(), 2);
        assert_eq!(on_global[0].0.symbols(), "dd");
        assert_eq!(on_global[1].0.symbols(), "yy");
        assert!(aliases.on(&Level::State("edit".into())).is_empty());
    }
}

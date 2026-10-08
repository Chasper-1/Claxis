use crate::arena::RecordId;

/// История правок: два стека записей арены.
///
/// Хранит только что произошло и в каком порядке. Ничего больше: отмена не
/// описывается отдельно, она выводится из самой записи — вставка отменяется
/// удалением, удаление отменяется возвратом отложенных сегментов в чтение.
#[derive(Debug, Default)]
pub struct History {
    undo_stack: Vec<RecordId>,
    redo_stack: Vec<RecordId>,
}

impl History {
    pub(crate) fn push(&mut self, id: RecordId) {
        self.undo_stack.push(id);
    }

    /// Отмена снимает вершину undo и кладёт её в redo. Стек — LIFO.
    pub(crate) fn undo(&mut self) -> Option<RecordId> {
        let id = self.undo_stack.pop()?;
        self.redo_stack.push(id);
        Some(id)
    }

    pub(crate) fn redo(&mut self) -> Option<RecordId> {
        let id = self.redo_stack.pop()?;
        self.undo_stack.push(id);
        Some(id)
    }

    /// Новая правка отменяет redo-ветку: документ пошёл по другой траектории.
    pub(crate) fn discard_redo(&mut self) {
        self.redo_stack.clear();
    }

    pub(crate) fn undo_stack(&self) -> &[RecordId] {
        &self.undo_stack
    }

    pub(crate) fn redo_stack(&self) -> &[RecordId] {
        &self.redo_stack
    }
}

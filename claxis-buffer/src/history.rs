use crate::current::Surgery;
use crate::edit::Edit;

#[derive(Debug)]
pub struct Record {
    pub edit: Edit,
    pub(crate) surgery: Surgery,
}

#[derive(Debug, Default)]
pub struct History {
    undo_stack: Vec<Record>,
    redo_stack: Vec<Record>,
}

impl History {
    pub(crate) fn record(&mut self, record: Record) {
        self.redo_stack.clear();
        self.undo_stack.push(record);
    }

    pub(crate) fn peek_undo(&self) -> Option<&Record> {
        self.undo_stack.last()
    }

    pub(crate) fn peek_redo(&self) -> Option<&Record> {
        self.redo_stack.last()
    }

    pub(crate) fn commit_undo(&mut self) {
        if let Some(record) = self.undo_stack.pop() {
            self.redo_stack.push(record);
        }
    }

    pub(crate) fn commit_redo(&mut self) {
        if let Some(record) = self.redo_stack.pop() {
            self.undo_stack.push(record);
        }
    }

    pub(crate) fn active(&self) -> impl Iterator<Item = Surgery> + '_ {
        self.undo_stack.iter().map(|record| record.surgery)
    }

    pub fn undo_stack(&self) -> &[Record] {
        &self.undo_stack
    }

    pub fn redo_stack(&self) -> &[Record] {
        &self.redo_stack
    }
}

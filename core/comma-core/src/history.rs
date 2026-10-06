//! Undo and redo. Every edit is a set of splices over the record list; applying a splice again
//! swaps the records back, so undo and redo are the same operation run in opposite orders.

use crate::document::Record;

pub(crate) struct Splice {
    at: usize,
    /// Number of records this splice currently occupies in the document.
    len: usize,
    /// Records that the next toggle puts back.
    other: Vec<Record>,
}

impl Splice {
    /// Replaces `remove` records at `at` with `insert` and returns the applied splice.
    pub(crate) fn apply(
        records: &mut Vec<Record>,
        at: usize,
        remove: usize,
        insert: Vec<Record>,
    ) -> Self {
        let len = insert.len();
        let other = records.splice(at..at + remove, insert).collect();
        Self { at, len, other }
    }

    fn toggle(&mut self, records: &mut Vec<Record>) {
        let incoming = std::mem::take(&mut self.other);
        let len = incoming.len();
        self.other = records.splice(self.at..self.at + self.len, incoming).collect();
        self.len = len;
    }
}

pub(crate) struct Transaction {
    pub splices: Vec<Splice>,
    pub columns_before: usize,
    pub columns_after: usize,
}

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}

impl History {
    pub fn record(&mut self, transaction: Transaction) {
        self.undo.push(transaction);
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the last transaction and returns the column count to restore.
    pub fn undo(&mut self, records: &mut Vec<Record>) -> Option<usize> {
        let mut transaction = self.undo.pop()?;
        for splice in transaction.splices.iter_mut().rev() {
            splice.toggle(records);
        }
        let columns = transaction.columns_before;
        self.redo.push(transaction);
        Some(columns)
    }

    /// Re-applies the last undone transaction and returns the column count to restore.
    pub fn redo(&mut self, records: &mut Vec<Record>) -> Option<usize> {
        let mut transaction = self.redo.pop()?;
        for splice in transaction.splices.iter_mut() {
            splice.toggle(records);
        }
        let columns = transaction.columns_after;
        self.undo.push(transaction);
        Some(columns)
    }
}

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
    /// Rows were added, removed or reordered, so row indices no longer line up.
    pub structural: bool,
    /// Whether the file ends with a line break, before and after.
    pub trailing_before: bool,
    pub trailing_after: bool,
}

/// What an undo or redo changed.
pub(crate) struct Reverted {
    pub columns: usize,
    pub structural: bool,
    pub trailing_newline: bool,
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

    /// Reverts the last transaction.
    pub fn undo(&mut self, records: &mut Vec<Record>) -> Option<Reverted> {
        let mut transaction = self.undo.pop()?;
        for splice in transaction.splices.iter_mut().rev() {
            splice.toggle(records);
        }
        let reverted = Reverted {
            columns: transaction.columns_before,
            structural: transaction.structural,
            trailing_newline: transaction.trailing_before,
        };
        self.redo.push(transaction);
        Some(reverted)
    }

    /// Re-applies the last undone transaction.
    pub fn redo(&mut self, records: &mut Vec<Record>) -> Option<Reverted> {
        let mut transaction = self.redo.pop()?;
        for splice in transaction.splices.iter_mut() {
            splice.toggle(records);
        }
        let reverted = Reverted {
            columns: transaction.columns_after,
            structural: transaction.structural,
            trailing_newline: transaction.trailing_after,
        };
        self.undo.push(transaction);
        Some(reverted)
    }
}

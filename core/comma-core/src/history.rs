//! Undo and redo. Every edit is a set of splices over the record list; applying a splice again
//! swaps the records back, so undo and redo are the same operation run in opposite orders.

use crate::document::Record;
use crate::edit::drop_later;

pub(crate) enum Splice {
    /// `len` records at `at`; `other` holds the records that the next toggle puts there instead.
    Range { at: usize, len: usize, other: Vec<Record> },
    /// Records replaced in place at `rows`; `other` holds the ones the next toggle swaps back.
    Swap { rows: Vec<usize>, other: Vec<Record> },
    /// Records at ascending `rows` (positions in the list that holds them) taken out of the list
    /// when `removed`, or put back in. One pass over the list either way, however scattered.
    Remove { rows: Vec<usize>, other: Vec<Record>, removed: bool },
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
        Self::Range { at, len, other }
    }

    /// Puts each record at its row, which must all differ, and returns the applied splice.
    pub(crate) fn swap(records: &mut [Record], updates: Vec<(usize, Record)>) -> Self {
        let (rows, other) = updates
            .into_iter()
            .map(|(row, record)| (row, std::mem::replace(&mut records[row], record)))
            .unzip();
        Self::Swap { rows, other }
    }

    /// Removes the records at ascending, distinct `rows` and returns the applied splice.
    pub(crate) fn remove(records: &mut Vec<Record>, rows: Vec<usize>) -> Self {
        let mut splice = Self::Remove { rows, other: Vec::new(), removed: false };
        splice.toggle(records);
        splice
    }

    /// Bytes the splice holds.
    pub(crate) fn heap_size(&self) -> usize {
        let (rows, other) = match self {
            Self::Range { other, .. } => (0, other),
            Self::Swap { rows, other } | Self::Remove { rows, other, .. } => (rows.capacity(), other),
        };
        rows * size_of::<usize>()
            + other.capacity() * size_of::<Record>()
            + other.iter().map(Record::heap_size).sum::<usize>()
    }

    fn toggle(&mut self, records: &mut Vec<Record>) {
        match self {
            Self::Range { at, len, other } => {
                let incoming = std::mem::take(other);
                let count = incoming.len();
                *other = records.splice(*at..*at + *len, incoming).collect();
                *len = count;
            }
            Self::Swap { rows, other } => {
                for (&row, record) in rows.iter().zip(other.iter_mut()) {
                    std::mem::swap(&mut records[row], record);
                }
            }
            Self::Remove { rows, other, removed } => {
                let old = std::mem::take(records);
                if *removed {
                    let mut back = std::mem::take(other).into_iter();
                    let mut kept = old.into_iter();
                    let mut next = rows.iter().peekable();
                    let total = kept.len() + back.len();
                    records.reserve_exact(total);
                    for position in 0..total {
                        let from_back = next.next_if(|&&row| row == position).is_some();
                        records.extend(if from_back { back.next() } else { kept.next() });
                    }
                } else {
                    records.reserve_exact(old.len() - rows.len());
                    other.reserve_exact(rows.len());
                    let mut next = rows.iter().peekable();
                    for (position, record) in old.into_iter().enumerate() {
                        if next.next_if(|&&row| row == position).is_some() {
                            other.push(record);
                        } else {
                            records.push(record);
                        }
                    }
                }
                *removed = !*removed;
            }
        }
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
    /// Bytes the splices hold; set when recorded.
    pub bytes: usize,
}

/// What an undo or redo changed.
pub(crate) struct Reverted {
    pub columns: usize,
    pub structural: bool,
    pub trailing_newline: bool,
}

/// Memory the undo history holds at most by default. A change to a whole large file keeps the
/// rows as they were, which can take gigabytes.
pub(crate) const UNDO_MEMORY_LIMIT: usize = 1 << 30;

pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Bytes held by the transactions in both lists, as counted when they were recorded.
    bytes: usize,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), bytes: 0, limit: UNDO_MEMORY_LIMIT }
    }
}

impl History {
    /// Adds a transaction. The oldest ones are dropped while the history holds more than its
    /// limit; the new one always stays.
    pub fn record(&mut self, mut transaction: Transaction) {
        transaction.bytes = transaction.splices.iter().map(Splice::heap_size).sum();
        self.bytes += transaction.bytes;
        self.undo.push(transaction);
        let redo = std::mem::take(&mut self.redo);
        if !redo.is_empty() {
            self.bytes -= redo.iter().map(|transaction| transaction.bytes).sum::<usize>();
            drop_later(redo);
        }
        self.trim();
    }

    pub fn set_limit(&mut self, bytes: usize) {
        self.limit = bytes;
        self.trim();
    }

    fn trim(&mut self) {
        let mut dropped = 0;
        while self.bytes > self.limit && self.undo.len() - dropped > 1 {
            self.bytes -= self.undo[dropped].bytes;
            dropped += 1;
        }
        if dropped > 0 {
            drop_later(self.undo.drain(..dropped).collect::<Vec<_>>());
        }
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

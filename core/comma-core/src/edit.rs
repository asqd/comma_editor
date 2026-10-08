//! Changes to many rows made off the document: computed on a snapshot, on all cores, with
//! progress and a way to cancel, then applied in one quick step. `Document::apply_edit` runs
//! the same code at once, for small files and tests.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::columns::TextTransform;
use crate::document::{Document, Field, Record};
use crate::error::Result;
use crate::formula::FormulaReport;
use crate::history::Splice;
use crate::range::CellRange;
use crate::search::SearchQuery;

/// A change to many rows; each is one undoable step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Replace every match in the visible rows.
    ReplaceAll { query: SearchQuery, replacement: String },
    /// Change the text of every cell in a range of visible cells.
    Transform { range: CellRange, transform: TextTransform },
    /// Fill the empty cells of a range with the nearest value above them, or with `value`.
    FillEmpty { range: CellRange, from_above: bool, value: String },
    /// Fix text read in the wrong encoding in a range, or in the whole file with the header.
    FixGarbled { range: Option<CellRange> },
    /// Insert a column named `name` before column `at` with the formula's value in every row.
    AddFormulaColumn { formula: String, name: String, at: usize },
    /// Write the formula's value into `column` of every visible row.
    FillWithFormula { formula: String, column: usize },
    /// Split a column into new columns after it.
    SplitColumn { column: usize, separator: String, regex: bool },
    /// Join columns into a new column after the last of them.
    MergeColumns { columns: Vec<usize>, separator: String },
    /// Delete visible rows that repeat an earlier one in `columns` (all when empty).
    RemoveDuplicates { columns: Vec<usize>, normalize: bool },
}

/// What an edit did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditOutcome {
    /// Whether the document changed; an undoable step was recorded.
    pub changed: bool,
    /// Cells changed; columns added by a split or a merge; rows removed as duplicates.
    pub count: usize,
    /// For formulas: the rows written and where the formula failed.
    pub report: Option<FormulaReport>,
}

/// How far an edit running off the document has come, and the way to cancel it.
#[derive(Debug, Default)]
pub struct Progress {
    cancelled: AtomicBool,
    done: AtomicUsize,
    total: AtomicUsize,
}

impl Progress {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Share of the work done, from 0 to 1.
    pub fn fraction(&self) -> f64 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0)
    }

    /// Adds `steps` to the work to do.
    pub(crate) fn expect(&self, steps: usize) {
        self.total.fetch_add(steps, Ordering::Relaxed);
    }

    pub(crate) fn advance(&self, steps: usize) {
        self.done.fetch_add(steps, Ordering::Relaxed);
    }
}

/// Rows handled between looks at `Progress`.
pub(crate) const BLOCK: usize = 16 * 1024;

/// `f` of every row in `rows`, on all cores, in order, without the `None`s. Each row counts as a
/// step of `progress`. `None` when cancelled.
pub(crate) fn map_rows<T: Send + 'static>(
    rows: Range<usize>,
    progress: &Progress,
    f: impl Fn(usize) -> Option<T> + Sync,
) -> Option<Vec<T>> {
    let starts: Vec<usize> = rows.clone().step_by(BLOCK).collect();
    let blocks: Vec<Option<Vec<T>>> = starts
        .into_par_iter()
        .map(|start| {
            if progress.is_cancelled() {
                return None;
            }
            let end = (start + BLOCK).min(rows.end);
            let items = (start..end).filter_map(&f).collect();
            progress.advance(end - start);
            Some(items)
        })
        .collect();
    if progress.is_cancelled() {
        drop_later(blocks);
        return None;
    }
    Some(blocks.into_iter().flatten().flatten().collect())
}

/// Frees `value` on another thread. Millions of rows take seconds to free, which would make
/// cancelling or a new change wait; freeing on several threads at once is slower still.
pub(crate) fn drop_later<T: Send + 'static>(value: T) {
    std::thread::spawn(move || drop(value));
}

/// A record with new values in some of its cells.
pub(crate) struct RowUpdate {
    pub row: usize,
    pub record: Record,
    pub changed: usize,
    /// One past the last changed column.
    pub width: usize,
}

/// The computed change, ready to apply.
pub(crate) enum Change {
    Nothing,
    /// Records replaced in place.
    Rows { updates: Vec<(usize, Record)>, columns_after: usize },
    /// Every record replaced, with new columns; the view starts over.
    All { records: Vec<Record>, columns_after: usize },
    /// Records removed.
    Remove(Vec<usize>),
}

pub(crate) struct Prepared {
    pub change: Change,
    pub outcome: EditOutcome,
}

impl Prepared {
    pub(crate) fn nothing() -> Self {
        Self { change: Change::Nothing, outcome: EditOutcome::default() }
    }

    /// Records with changed cells of a document with `columns` columns.
    pub(crate) fn rows(updates: Vec<RowUpdate>, columns: usize, report: Option<FormulaReport>) -> Self {
        let count = updates.iter().map(|update| update.changed).sum();
        let columns_after = updates.iter().map(|update| update.width).fold(columns, usize::max);
        let updates = updates.into_iter().map(|update| (update.row, update.record)).collect();
        Self {
            change: Change::Rows { updates, columns_after },
            outcome: EditOutcome { changed: false, count, report },
        }
    }
}

/// An edit computed off the document; see `Document::begin_edit`.
pub struct EditTask {
    edit: Edit,
    snapshot: Document,
    generation: u64,
    prepared: Option<Result<Prepared>>,
}

impl EditTask {
    /// Computes the change; call off the main thread. Returns `false` when cancelled.
    pub fn run(&mut self, progress: &Progress) -> bool {
        match self.snapshot.prepare_edit(&self.edit, progress).transpose() {
            Some(prepared) => {
                self.prepared = Some(prepared);
                true
            }
            None => false,
        }
    }
}

impl Document {
    /// Makes an edit at once.
    pub fn apply_edit(&mut self, edit: &Edit) -> Result<EditOutcome> {
        let prepared = self.prepare_edit(edit, &Progress::default())?;
        Ok(prepared.map_or_else(EditOutcome::default, |prepared| self.apply_prepared(prepared)))
    }

    /// Prepares an edit to be computed with `EditTask::run` off the document, which stays free
    /// for the table meanwhile, and applied with `finish_edit`.
    pub fn begin_edit(&self, edit: Edit) -> EditTask {
        EditTask {
            edit,
            snapshot: self.snapshot(),
            generation: self.generation,
            prepared: None,
        }
    }

    /// Applies a computed edit as one undoable step. `Ok(None)` when it was cancelled or the
    /// rows changed since it began; then nothing changes.
    pub fn finish_edit(&mut self, task: EditTask) -> Result<Option<EditOutcome>> {
        let EditTask { snapshot, generation, prepared, .. } = task;
        // Without the snapshot the rows change in place instead of being copied first.
        drop(snapshot);
        let Some(prepared) = prepared.transpose()? else { return Ok(None) };
        if generation != self.generation {
            return Ok(None);
        }
        Ok(Some(self.apply_prepared(prepared)))
    }

    /// The change an edit makes; `Ok(None)` when cancelled.
    fn prepare_edit(&self, edit: &Edit, progress: &Progress) -> Result<Option<Prepared>> {
        match edit {
            Edit::ReplaceAll { query, replacement } => self.prepare_replace_all(query, replacement, progress),
            Edit::Transform { range, transform } => {
                self.check_range(range)?;
                let columns: Vec<usize> = range.columns().collect();
                Ok(self.cell_updates(range.rows(), &columns, progress, |_, _, value| Some(transform.apply(value))))
            }
            Edit::FillEmpty { range, from_above, value } => self.prepare_fill_empty(*range, *from_above, value, progress),
            Edit::FixGarbled { range } => self.prepare_fix_garbled(*range, progress),
            Edit::AddFormulaColumn { formula, name, at } => self.prepare_formula_column(formula, name, *at, progress),
            Edit::FillWithFormula { formula, column } => self.prepare_formula_fill(formula, *column, progress),
            Edit::SplitColumn { column, separator, regex } => self.prepare_split(*column, separator, *regex, progress),
            Edit::MergeColumns { columns, separator } => self.prepare_merge(columns, separator, progress),
            Edit::RemoveDuplicates { columns, normalize } => self.prepare_duplicates(columns, *normalize, progress),
        }
    }

    pub(crate) fn apply_prepared(&mut self, prepared: Prepared) -> EditOutcome {
        let changed = match prepared.change {
            Change::Nothing => false,
            Change::Rows { updates, .. } if updates.is_empty() => false,
            Change::Rows { updates, columns_after } => {
                let splice = Splice::swap(Arc::make_mut(&mut self.records).as_mut_slice(), updates);
                self.commit(vec![splice], columns_after, false);
                true
            }
            Change::All { records, columns_after } => {
                self.replace_records(records, columns_after, false);
                self.reset_view();
                true
            }
            Change::Remove(rows) if rows.is_empty() => false,
            Change::Remove(rows) => {
                self.delete_rows(&rows).expect("rows of this document");
                true
            }
        };
        EditOutcome { changed, ..prepared.outcome }
    }

    /// Visible rows whose cells in `columns` change: `value(visible row, column, current)` is a
    /// cell's new value, `None` keeps it. `None` when cancelled.
    pub(crate) fn cell_updates(
        &self,
        rows: Range<usize>,
        columns: &[usize],
        progress: &Progress,
        value: impl Fn(usize, usize, &str) -> Option<String> + Sync,
    ) -> Option<Prepared> {
        progress.expect(rows.len());
        let updates = map_rows(rows, progress, |row| {
            let record = self.visible_record(row)?;
            self.updated_record(record, columns.iter().copied(), |column, current| value(row, column, current))
        })?;
        Some(Prepared::rows(updates, self.column_count(), None))
    }

    /// Record `row` with `value(column, current)` in `columns` (`None` keeps a cell), or `None`
    /// when no cell changes.
    pub(crate) fn updated_record(
        &self,
        row: usize,
        columns: impl Iterator<Item = usize>,
        mut value: impl FnMut(usize, &str) -> Option<String>,
    ) -> Option<RowUpdate> {
        let mut fields: Option<Vec<Field>> = None;
        let (mut changed, mut width) = (0, 0);
        for column in columns {
            let current = self.value_at(row, column);
            let Some(new_value) = value(column, &current) else { continue };
            if current == new_value {
                continue;
            }
            let fields = fields.get_or_insert_with(|| self.fields(&self.records[row]));
            if fields.len() <= column {
                fields.resize(column + 1, Field::Text(String::new()));
            }
            fields[column] = Field::Text(new_value);
            width = width.max(column + 1);
            changed += 1;
        }
        Some(RowUpdate {
            row,
            record: Record::modified(fields?, self.records[row].term()),
            changed,
            width,
        })
    }
}

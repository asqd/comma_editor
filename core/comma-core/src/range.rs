//! Rectangular ranges of visible cells: copy, paste, fill and clear, each one undoable step.

use std::ops::Range;
use std::sync::Arc;

use crate::document::{Document, Field, Record};
use crate::edit::Progress;
use crate::error::{Error, Result};
use crate::history::Splice;
use crate::parser;
use crate::view::parse_number;

/// Visible rows `top..=bottom` and columns `left..=right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
}

impl CellRange {
    pub(crate) fn rows(&self) -> Range<usize> {
        self.top..self.bottom + 1
    }

    pub(crate) fn columns(&self) -> Range<usize> {
        self.left..self.right + 1
    }
}

/// Summary of a range for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeStats {
    pub cells: usize,
    /// Cells that are not empty.
    pub filled: usize,
    /// Cells holding a number, and their sum.
    pub numbers: usize,
    pub sum: f64,
}

impl Document {
    /// The range as tab-separated text, the way spreadsheets put cells on the clipboard.
    pub fn copy_range(&self, range: CellRange) -> Result<String> {
        self.check_range(&range)?;
        let mut text = String::new();
        for row in range.rows() {
            let record = self.visible_record_checked(row)?;
            for column in range.columns() {
                if column > range.left {
                    text.push('\t');
                }
                let value = self.value_at(record, column);
                if value.contains(['\t', '\n', '\r', '"']) {
                    text.push('"');
                    text.push_str(&value.replace('"', "\"\""));
                    text.push('"');
                } else {
                    text.push_str(&value);
                }
            }
            text.push('\n');
        }
        Ok(text)
    }

    pub fn range_stats(&self, range: CellRange) -> Result<RangeStats> {
        self.check_range(&range)?;
        let mut stats = RangeStats::default();
        for row in range.rows() {
            let record = self.visible_record_checked(row)?;
            for column in range.columns() {
                let value = self.value_at(record, column);
                stats.cells += 1;
                if value.trim().is_empty() {
                    continue;
                }
                stats.filled += 1;
                if let Some(number) = parse_number(&value) {
                    stats.numbers += 1;
                    stats.sum += number;
                }
            }
        }
        Ok(stats)
    }

    /// Sets every cell of the range to `value`; an empty value clears the range.
    /// Returns the number of cells that changed.
    pub fn fill_range(&mut self, range: CellRange, value: &str) -> Result<usize> {
        self.check_range(&range)?;
        let columns: Vec<usize> = range.columns().collect();
        let prepared = self
            .cell_updates(range.rows(), &columns, &Progress::default(), |_, _, _| Some(value.to_owned()))
            .expect("nothing cancels it");
        Ok(self.apply_prepared(prepared).count)
    }

    /// Copies the top row of the range into the rows below it.
    pub fn fill_down(&mut self, range: CellRange) -> Result<usize> {
        self.check_range(&range)?;
        let top = self.visible_record_checked(range.top)?;
        let source: Vec<String> = range
            .columns()
            .map(|column| self.value_at(top, column).into_owned())
            .collect();
        Ok(self.edit_range(
            Vec::new(),
            range.top + 1..range.bottom + 1,
            range.columns(),
            false,
            |_, column, _| Some(source[column - range.left].clone()),
        ))
    }

    /// Writes `grid` with its top-left cell at (`top`, `left`). Rows past the end are appended
    /// when the rows are neither sorted nor filtered, otherwise the grid is cut at the last
    /// visible row; columns past the end are added. Returns the range written, if any.
    pub fn paste(&mut self, top: usize, left: usize, grid: &[Vec<String>]) -> Result<Option<CellRange>> {
        let height = grid.len();
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        let visible = self.visible_row_count();
        if top > visible {
            return Err(Error::RowOutOfRange { row: top, rows: visible });
        }
        if left > self.column_count() {
            return Err(Error::ColumnOutOfRange {
                column: left,
                columns: self.column_count(),
            });
        }
        if height == 0 || width == 0 {
            return Ok(None);
        }

        let mut splices = Vec::new();
        let mut bottom = (top + height).min(visible);
        if top + height > visible && !self.has_view() {
            let columns = self.column_count().max(1);
            let term = self.dialect.line_ending.terminator();
            let rows = (visible..top + height)
                .map(|_| Record::modified(vec![Field::Text(String::new()); columns], term))
                .collect();
            let at = self.records.len();
            splices.push(Splice::apply(Arc::make_mut(&mut self.records), at, 0, rows));
            bottom = top + height;
        }
        if bottom == top {
            return Ok(None);
        }
        let structural = !splices.is_empty();
        self.edit_range(splices, top..bottom, left..left + width, structural, |row, column, _| {
            grid[row - top].get(column - left).cloned()
        });
        Ok(Some(CellRange {
            top,
            left,
            bottom: bottom - 1,
            right: left + width - 1,
        }))
    }

    /// Applies `value(row, column, current)` to the cells of visible `rows` × `columns`, row by
    /// row (`None` keeps a cell), and records everything, with `splices` already applied, as one
    /// undoable step.
    pub(crate) fn edit_range(
        &mut self,
        mut splices: Vec<Splice>,
        rows: Range<usize>,
        columns: Range<usize>,
        structural: bool,
        mut value: impl FnMut(usize, usize, &str) -> Option<String>,
    ) -> usize {
        let mut changed = 0;
        let mut columns_after = self.column_count();
        for row in rows {
            let Some(record) = self.visible_record(row) else { continue };
            let mut fields: Option<Vec<Field>> = None;
            for column in columns.clone() {
                let current = self.value_at(record, column);
                let Some(new_value) = value(row, column, &current) else { continue };
                if current == new_value {
                    continue;
                }
                drop(current);
                let fields = fields.get_or_insert_with(|| self.fields(&self.records[record]));
                if fields.len() <= column {
                    fields.resize(column + 1, Field::Text(String::new()));
                }
                fields[column] = Field::Text(new_value);
                columns_after = columns_after.max(column + 1);
                changed += 1;
            }
            if let Some(fields) = fields {
                let updated = Record::modified(fields, self.records[record].term());
                splices.push(Splice::apply(Arc::make_mut(&mut self.records), record, 1, vec![updated]));
            }
        }
        if !splices.is_empty() {
            self.commit(splices, columns_after, structural);
        }
        changed
    }

    pub(crate) fn check_range(&self, range: &CellRange) -> Result<()> {
        if range.top > range.bottom || range.bottom >= self.visible_row_count() {
            return Err(Error::RowOutOfRange {
                row: range.bottom,
                rows: self.visible_row_count(),
            });
        }
        if range.left > range.right || range.right >= self.column_count() {
            return Err(Error::ColumnOutOfRange {
                column: range.right,
                columns: self.column_count(),
            });
        }
        Ok(())
    }
}

/// Cells from clipboard text. Tab-separated text (what spreadsheets copy) is split into cells,
/// with quoted values that hold tabs or line breaks; other text gives one cell per line.
pub fn parse_pasted_text(text: &str) -> Vec<Vec<String>> {
    let quoted_cell = text.len() > 1 && text.starts_with('"') && text.trim_end().ends_with('"');
    if !text.contains('\t') && !quoted_cell {
        return text.lines().map(|line| vec![line.to_owned()]).collect();
    }
    let bytes = text.as_bytes();
    parser::scan(bytes, 0, b'\t', b'"', None)
        .records
        .iter()
        .map(|record| {
            let line = &bytes[record.start..record.end];
            parser::split_fields(line, b'\t', b'"')
                .into_iter()
                .map(|(start, end)| {
                    String::from_utf8_lossy(&parser::unquote(&line[start..end], b'"')).into_owned()
                })
                .collect()
        })
        .collect()
}

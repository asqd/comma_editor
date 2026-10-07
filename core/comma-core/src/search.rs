//! Find and replace over the visible rows, in the order they are shown.

use std::sync::Arc;
use std::ops::Range;

use rayon::prelude::*;
use regex::{NoExpand, Regex, RegexBuilder};

use crate::document::{Document, Field, Record};
use crate::error::{Error, Result};
use crate::history::Splice;
use crate::range::CellRange;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    /// The whole value must match, not just a part of it.
    pub whole_cell: bool,
    /// `text` is a regular expression; replacements may use `$1`, `${name}`.
    pub regex: bool,
    /// Search one column only.
    pub column: Option<usize>,
    /// Search these cells only (visible rows); combines with `column`.
    pub range: Option<CellRange>,
}

/// What a query looks at: the matcher, the visible rows and the columns.
struct Scope {
    matcher: Matcher,
    rows: Range<usize>,
    columns: Vec<usize>,
}

struct Matcher {
    regex: Regex,
    literal: bool,
}

impl Matcher {
    fn new(query: &SearchQuery) -> Result<Self> {
        let pattern = if query.regex {
            query.text.clone()
        } else {
            regex::escape(&query.text)
        };
        let pattern = if query.whole_cell {
            format!("^(?:{pattern})$")
        } else {
            pattern
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!query.case_sensitive)
            .build()
            .map_err(|error| Error::InvalidPattern(error.to_string()))?;
        Ok(Self {
            regex,
            literal: !query.regex,
        })
    }

    fn is_match(&self, value: &str) -> bool {
        self.regex.is_match(value)
    }

    fn replace(&self, value: &str, replacement: &str) -> String {
        if self.literal {
            self.regex.replace_all(value, NoExpand(replacement)).into_owned()
        } else {
            self.regex.replace_all(value, replacement).into_owned()
        }
    }
}

impl Document {
    /// Next matching cell after `from` (or before it when `forward` is false), wrapping around.
    /// Positions are `(visible row, column)`. Without `from`, starts at the first or last cell.
    pub fn find(
        &self,
        query: &SearchQuery,
        from: Option<(usize, usize)>,
        forward: bool,
    ) -> Result<Option<(usize, usize)>> {
        let Some(Scope { matcher, rows: scope, columns }) = self.prepare(query)? else {
            return Ok(None);
        };
        // Rows are counted from the start of the scope, so wrapping stays inside it.
        let (first, rows) = (scope.start, scope.len());
        if rows == 0 {
            return Ok(None);
        }
        let scan = |row: usize, accept: &(dyn Fn(usize) -> bool + Sync)| -> Option<(usize, usize)> {
            let row = first + row;
            let record = self.visible_record(row)?;
            let hit = |&column: &usize| accept(column) && matcher.is_match(&self.value_at(record, column));
            let column = if forward {
                columns.iter().find(|c| hit(c))
            } else {
                columns.iter().rev().find(|c| hit(c))
            };
            column.map(|&column| (row, column))
        };

        // The starting row is split: cells after `from` come first, cells up to it last.
        let (start, column) = match from {
            Some((row, column)) if scope.contains(&row) => (row - first, column as i64),
            _ if forward => (rows - 1, i64::MAX),
            _ => (0, -1),
        };
        let after = |c: usize| if forward { (c as i64) > column } else { (c as i64) < column };
        if let Some(hit) = scan(start, &after) {
            return Ok(Some(hit));
        }
        // The other rows in order, searched on all cores; the first match in that order wins.
        let row_at = |step: usize| if forward { (start + step) % rows } else { (start + rows - step) % rows };
        let hit = (1..rows)
            .into_par_iter()
            .with_min_len(256)
            .find_map_first(|step| scan(row_at(step), &|_| true));
        if hit.is_some() {
            return Ok(hit);
        }
        Ok(scan(start, &|c| !after(c)))
    }

    /// Number of matching cells in the visible rows.
    pub fn count_matches(&self, query: &SearchQuery) -> Result<usize> {
        let Some(Scope { matcher, rows, columns }) = self.prepare(query)? else {
            return Ok(0);
        };
        Ok(rows
            .into_par_iter()
            .filter_map(|row| self.visible_record(row))
            .map(|record| {
                columns
                    .iter()
                    .filter(|&&column| matcher.is_match(&self.value_at(record, column)))
                    .count()
            })
            .sum())
    }

    /// Replaces the matches in one cell. Returns `false` if the cell does not match.
    pub fn replace_in_cell(
        &mut self,
        row: usize,
        column: usize,
        query: &SearchQuery,
        replacement: &str,
    ) -> Result<bool> {
        let Some(Scope { matcher, .. }) = self.prepare(query)? else {
            return Ok(false);
        };
        let record = self.visible_record_checked(row)?;
        let value = self.value_at(record, column);
        if !matcher.is_match(&value) {
            return Ok(false);
        }
        let replaced = matcher.replace(&value, replacement);
        self.set_cell(record, column, &replaced)
    }

    /// Replaces every match in the visible rows as one undoable step.
    /// Returns the number of cells that changed.
    pub fn replace_all(&mut self, query: &SearchQuery, replacement: &str) -> Result<usize> {
        let Some(Scope { matcher, rows, columns }) = self.prepare(query)? else {
            return Ok(0);
        };
        // New values are found on all cores; the changed rows are then replaced in order.
        let updates: Vec<(usize, Record, usize)> = rows
            .into_par_iter()
            .filter_map(|row| {
                let record = self.visible_record(row)?;
                let mut fields: Option<Vec<Field>> = None;
                let mut changed = 0;
                for &column in &columns {
                    let value = self.value_at(record, column);
                    if !matcher.is_match(&value) {
                        continue;
                    }
                    let replaced = matcher.replace(&value, replacement);
                    if replaced == value {
                        continue;
                    }
                    let fields = fields.get_or_insert_with(|| self.fields(&self.records[record]));
                    if fields.len() <= column {
                        fields.resize(column + 1, Field::Text(String::new()));
                    }
                    fields[column] = Field::Text(replaced);
                    changed += 1;
                }
                fields.map(|fields| (record, Record::modified(fields, self.records[record].term()), changed))
            })
            .collect();
        let changed = updates.iter().map(|(_, _, changed)| changed).sum();
        let records = Arc::make_mut(&mut self.records);
        let splices: Vec<Splice> = updates
            .into_iter()
            .map(|(record, updated, _)| Splice::apply(records, record, 1, vec![updated]))
            .collect();
        if !splices.is_empty() {
            self.commit(splices, self.column_count(), false);
        }
        Ok(changed)
    }

    /// What to search, or `None` when there is nothing to look for. A range that reaches past
    /// the table is cut to it.
    fn prepare(&self, query: &SearchQuery) -> Result<Option<Scope>> {
        if query.text.is_empty() && !query.whole_cell {
            return Ok(None);
        }
        let matcher = Matcher::new(query)?;
        let (mut rows, mut columns) = (0..self.visible_row_count(), 0..self.column_count());
        if let Some(range) = &query.range {
            rows = range.top..(range.bottom + 1).min(rows.end);
            columns = range.left..(range.right + 1).min(columns.end);
        }
        let columns: Vec<usize> = match query.column {
            Some(column) => columns.filter(|&c| c == column).collect(),
            None => columns.collect(),
        };
        if columns.is_empty() || rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(Scope { matcher, rows, columns }))
    }
}

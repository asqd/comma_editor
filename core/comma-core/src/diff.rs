//! Comparing two tables: rows matched by key columns (or by position), rows found in only one
//! of them, and matched rows whose values differ. Columns are matched by name when both tables
//! have a header row, by position otherwise.
//!
//! Keys of both tables are made and sorted on all cores and then merged in one pass, so tables
//! of millions of rows compare in seconds. Rows that repeat a key are paired in file order.

use std::borrow::Cow;
use std::cmp::Ordering;

use rayon::prelude::*;

use crate::document::Document;
use crate::edit::{Progress, drop_later, map_rows};
use crate::error::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// Key columns of the left table; empty compares rows by position.
    pub key: Vec<usize>,
    /// Values that differ only in case are equal, in keys too.
    pub ignore_case: bool,
    /// Values that differ only in spaces around them are equal, in keys too.
    pub trim: bool,
}

/// A column of the comparison: where it is in each table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffColumn {
    /// The header of the column, from the left table if it has one there.
    pub name: Option<String>,
    pub left: Option<usize>,
    pub right: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffStatus {
    OnlyLeft,
    OnlyRight,
    Changed,
    Same,
}

/// One row of the comparison; rows are rows of the files, header included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffEntry {
    pub status: DiffStatus,
    pub left: Option<usize>,
    pub right: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffCounts {
    pub only_left: usize,
    pub only_right: usize,
    pub changed: usize,
    pub same: usize,
    /// Rows whose key an earlier row of the same table already has.
    pub repeated_left: usize,
    pub repeated_right: usize,
}

/// The values of one column in a row of the comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCell {
    pub left: Option<String>,
    pub right: Option<String>,
    /// Both rows have the column and its values differ.
    pub changed: bool,
}

pub struct Diff {
    left: Document,
    right: Document,
    columns: Vec<DiffColumn>,
    entries: Vec<DiffEntry>,
    counts: DiffCounts,
    options: DiffOptions,
}

impl Diff {
    pub fn columns(&self) -> &[DiffColumn] {
        &self.columns
    }

    /// Rows of the left table in its order, each with its match, then rows only in the right.
    pub fn entries(&self) -> &[DiffEntry] {
        &self.entries
    }

    pub fn counts(&self) -> &DiffCounts {
        &self.counts
    }

    pub fn cells(&self, entry: &DiffEntry) -> Vec<DiffCell> {
        self.columns
            .iter()
            .map(|column| {
                let left = column.left.zip(entry.left).map(|(c, row)| self.left.value_at(row, c).into_owned());
                let right = column.right.zip(entry.right).map(|(c, row)| self.right.value_at(row, c).into_owned());
                let changed = match (&left, &right) {
                    (Some(left), Some(right)) => !self.equal(left, right),
                    _ => false,
                };
                DiffCell { left, right, changed }
            })
            .collect()
    }

    /// The comparison as a CSV table: a status column named by `names` (only left, only right,
    /// changed, same), then every column; a changed value reads "left → right".
    pub fn to_csv(&self, statuses: &[DiffStatus], names: [&str; 4], status_title: &str) -> Vec<u8> {
        let mut out = String::new();
        let header: Vec<String> = std::iter::once(status_title.to_owned())
            .chain(self.columns.iter().enumerate().map(|(index, column)| {
                column.name.clone().unwrap_or_else(|| format!("{}", index + 1))
            }))
            .collect();
        push_csv_row(&mut out, &header);
        for entry in self.entries.iter().filter(|entry| statuses.contains(&entry.status)) {
            let name = match entry.status {
                DiffStatus::OnlyLeft => names[0],
                DiffStatus::OnlyRight => names[1],
                DiffStatus::Changed => names[2],
                DiffStatus::Same => names[3],
            };
            let row: Vec<String> = std::iter::once(name.to_owned())
                .chain(self.cells(entry).into_iter().map(|cell| match (cell.left, cell.right) {
                    (Some(left), Some(right)) if cell.changed => format!("{left} → {right}"),
                    (Some(value), _) | (None, Some(value)) => value,
                    (None, None) => String::new(),
                }))
                .collect();
            push_csv_row(&mut out, &row);
        }
        out.into_bytes()
    }

    fn equal(&self, left: &str, right: &str) -> bool {
        left == right || normalize(left, &self.options) == normalize(right, &self.options)
    }
}

fn push_csv_row(out: &mut String, values: &[String]) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        if value.contains([',', '"', '\n', '\r']) || value.trim() != value {
            out.push('"');
            out.push_str(&value.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(value);
        }
    }
    out.push('\n');
}

fn normalize<'a>(value: &'a str, options: &DiffOptions) -> Cow<'a, str> {
    let value = if options.trim { value.trim() } else { value };
    if options.ignore_case { Cow::Owned(value.to_lowercase()) } else { Cow::Borrowed(value) }
}

/// Separates the values of a key made of several columns.
const KEY_SEPARATOR: char = '\u{1F}';

impl Document {
    /// How the columns of this table and `other` line up: by name when both have a header row,
    /// by position otherwise. Columns only in `other` come last.
    pub fn diff_columns(&self, other: &Document) -> Vec<DiffColumn> {
        let names = |document: &Document| -> Option<Vec<String>> {
            document.has_header().then(|| {
                (0..document.column_count()).map(|c| document.value_at(0, c).into_owned()).collect()
            })
        };
        let (Some(left), Some(right)) = (names(self), names(other)) else {
            return (0..self.column_count().max(other.column_count()))
                .map(|c| DiffColumn {
                    name: None,
                    left: (c < self.column_count()).then_some(c),
                    right: (c < other.column_count()).then_some(c),
                })
                .collect();
        };
        let mut used = vec![false; right.len()];
        let mut find = |name: &str, loose: bool| -> Option<usize> {
            let key = |text: &str| if loose { text.trim().to_lowercase() } else { text.to_owned() };
            let wanted = key(name);
            let found = right.iter().enumerate().position(|(c, title)| !used[c] && key(title) == wanted)?;
            used[found] = true;
            Some(found)
        };
        // Exact names first, so a loose match does not take a column another one names exactly.
        let exact: Vec<Option<usize>> = left.iter().map(|name| find(name, false)).collect();
        let mut columns: Vec<DiffColumn> = left
            .iter()
            .zip(exact)
            .enumerate()
            .map(|(c, (name, right))| DiffColumn { name: Some(name.clone()), left: Some(c), right })
            .collect();
        for column in columns.iter_mut().filter(|column| column.right.is_none()) {
            column.right = find(column.name.as_deref().unwrap_or_default(), true);
        }
        columns.extend(
            right
                .iter()
                .enumerate()
                .filter(|&(c, _)| !used[c])
                .map(|(c, name)| DiffColumn { name: Some(name.clone()), left: None, right: Some(c) }),
        );
        columns
    }

    /// Columns of this table that make a good key against `other`: a column both tables name
    /// alike whose values are filled and unique in the first rows of each, preferring names such
    /// as "id". Empty when there is none; rows are then compared by position.
    pub fn suggest_diff_key(&self, other: &Document) -> Vec<usize> {
        const SAMPLE: usize = 10_000;
        if !self.has_header() || !other.has_header() {
            return Vec::new();
        }
        let unique = |document: &Document, column: usize| -> bool {
            let rows = document.header_offset()..document.row_count().min(SAMPLE + 1);
            let mut seen = std::collections::HashSet::with_capacity(rows.len());
            !rows.is_empty() && rows.into_iter().all(|row| {
                let value = document.value_at(row, column);
                !value.trim().is_empty() && seen.insert(value)
            })
        };
        let id_like = |name: &str| {
            let name = name.trim().to_lowercase();
            ["id", "key", "uuid", "guid", "sku", "code", "код", "ключ", "артикул", "номер", "№"]
                .iter()
                .any(|word| name == *word || name.ends_with(&format!("_{word}")) || name.ends_with(&format!(" {word}")))
        };
        let candidates: Vec<DiffColumn> = self
            .diff_columns(other)
            .into_iter()
            .filter(|column| column.left.is_some() && column.right.is_some())
            .collect();
        let mut ordered: Vec<&DiffColumn> = candidates.iter().filter(|c| id_like(c.name.as_deref().unwrap_or(""))).collect();
        ordered.extend(candidates.iter().filter(|c| !id_like(c.name.as_deref().unwrap_or(""))));
        ordered
            .into_iter()
            .find(|column| unique(self, column.left.unwrap()) && unique(other, column.right.unwrap()))
            .map(|column| vec![column.left.unwrap()])
            .unwrap_or_default()
    }

    /// Compares the data rows of this table (left) with `other` (right). `Ok(None)` when
    /// cancelled.
    pub fn diff(&self, other: &Document, options: &DiffOptions, progress: &Progress) -> Result<Option<Diff>> {
        let columns = self.diff_columns(other);
        let key: Vec<(usize, usize)> = options
            .key
            .iter()
            .map(|&left| {
                columns
                    .iter()
                    .find(|column| column.left == Some(left))
                    .and_then(|column| column.right.map(|right| (left, right)))
                    .ok_or(Error::ColumnOutOfRange { column: left, columns: self.column_count() })
            })
            .collect::<Result<_>>()?;
        let left_rows = self.header_offset()..self.row_count();
        let right_rows = other.header_offset()..other.row_count();
        progress.expect(left_rows.len() + right_rows.len());

        let mut counts = DiffCounts::default();
        let pairs: Vec<(Option<usize>, Option<usize>)> = if key.is_empty() {
            let (left, right) = (left_rows.len(), right_rows.len());
            progress.advance(left + right);
            (0..left.max(right))
                .map(|n| ((n < left).then(|| left_rows.start + n), (n < right).then(|| right_rows.start + n)))
                .collect()
        } else {
            let make_key = |document: &Document, row: usize, columns: &mut dyn Iterator<Item = usize>| -> String {
                let mut text = String::new();
                for (index, column) in columns.enumerate() {
                    if index > 0 {
                        text.push(KEY_SEPARATOR);
                    }
                    text.push_str(&normalize(&document.value_at(row, column), options));
                }
                text
            };
            let Some(mut left) = map_rows(left_rows, progress, |row| {
                Some((make_key(self, row, &mut key.iter().map(|k| k.0)), row))
            }) else {
                return Ok(None);
            };
            let Some(mut right) = map_rows(right_rows, progress, |row| {
                Some((make_key(other, row, &mut key.iter().map(|k| k.1)), row))
            }) else {
                drop_later(left);
                return Ok(None);
            };
            left.par_sort_unstable();
            right.par_sort_unstable();
            if progress.is_cancelled() {
                drop_later((left, right));
                return Ok(None);
            }
            let pairs = merge(&left, &right, &mut counts);
            drop_later((left, right));
            pairs
        };

        // Matched rows are compared on all cores.
        let compare = |left: usize, right: usize| -> bool {
            columns.iter().any(|column| match (column.left, column.right) {
                (Some(l), Some(r)) => {
                    let (a, b) = (self.value_at(left, l), other.value_at(right, r));
                    a != b && normalize(&a, options) != normalize(&b, options)
                }
                _ => false,
            })
        };
        let mut entries: Vec<DiffEntry> = pairs
            .into_par_iter()
            .map(|pair| match pair {
                (Some(left), Some(right)) => DiffEntry {
                    status: if compare(left, right) { DiffStatus::Changed } else { DiffStatus::Same },
                    left: Some(left),
                    right: Some(right),
                },
                (Some(left), None) => DiffEntry { status: DiffStatus::OnlyLeft, left: Some(left), right: None },
                (None, right) => DiffEntry { status: DiffStatus::OnlyRight, left: None, right },
            })
            .collect();
        if progress.is_cancelled() {
            return Ok(None);
        }
        // Left rows in file order, then the rows only in the right table in theirs.
        entries.par_sort_unstable_by(|a, b| match (a.left, b.left) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.right.cmp(&b.right),
        });
        for entry in &entries {
            match entry.status {
                DiffStatus::OnlyLeft => counts.only_left += 1,
                DiffStatus::OnlyRight => counts.only_right += 1,
                DiffStatus::Changed => counts.changed += 1,
                DiffStatus::Same => counts.same += 1,
            }
        }
        Ok(Some(Diff {
            left: self.snapshot(),
            right: other.snapshot(),
            columns,
            entries,
            counts,
            options: options.clone(),
        }))
    }
}

/// Pairs rows of two key lists sorted by key: the n-th row with a key in one list goes with
/// the n-th row with that key in the other.
fn merge(left: &[(String, usize)], right: &[(String, usize)], counts: &mut DiffCounts) -> Vec<(Option<usize>, Option<usize>)> {
    let mut pairs = Vec::with_capacity(left.len().max(right.len()));
    let (mut i, mut j) = (0, 0);
    while i < left.len() || j < right.len() {
        let order = match (left.get(i), right.get(j)) {
            (Some(a), Some(b)) => a.0.cmp(&b.0),
            (Some(_), None) => Ordering::Less,
            _ => Ordering::Greater,
        };
        let key = if order == Ordering::Greater { &right[j].0 } else { &left[i].0 };
        let group = |list: &[(String, usize)], start: usize| {
            list[start..].iter().take_while(|(k, _)| k == key).count()
        };
        let (a, b) = match order {
            Ordering::Less => (group(left, i), 0),
            Ordering::Greater => (0, group(right, j)),
            Ordering::Equal => (group(left, i), group(right, j)),
        };
        counts.repeated_left += a.saturating_sub(1);
        counts.repeated_right += b.saturating_sub(1);
        for n in 0..a.max(b) {
            pairs.push(((n < a).then(|| left[i + n].1), (n < b).then(|| right[j + n].1)));
        }
        i += a;
        j += b;
    }
    pairs
}

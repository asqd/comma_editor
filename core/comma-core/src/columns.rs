//! Column operations: text transforms, filling blanks, splitting and merging columns,
//! removing duplicate rows. Each is one undoable step.

use std::collections::HashSet;

use regex::Regex;

use crate::document::{Document, Field};
use crate::edit::{BLOCK, Change, Edit, EditOutcome, Prepared, Progress, drop_later, map_rows};
use crate::error::{Error, Result, SheetStructure};
use crate::range::CellRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextTransform {
    /// Remove spaces at both ends.
    Trim,
    /// Trim and turn every run of whitespace into one space.
    CollapseSpaces,
    Uppercase,
    Lowercase,
    /// First letter of every word upper case, the rest lower case.
    Capitalize,
}

impl TextTransform {
    pub fn apply(self, value: &str) -> String {
        match self {
            Self::Trim => value.trim().to_owned(),
            Self::CollapseSpaces => value.split_whitespace().collect::<Vec<_>>().join(" "),
            Self::Uppercase => value.to_uppercase(),
            Self::Lowercase => value.to_lowercase(),
            Self::Capitalize => {
                let mut out = String::with_capacity(value.len());
                let mut word_start = true;
                for c in value.chars() {
                    if c.is_alphanumeric() {
                        if word_start {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        word_start = false;
                    } else {
                        out.push(c);
                        word_start = c != '\'' && c != '’';
                    }
                }
                out
            }
        }
    }
}

impl Document {
    /// Transforms the text of every cell in the range. Returns the number of changed cells.
    pub fn transform_range(&mut self, range: CellRange, transform: TextTransform) -> Result<usize> {
        Ok(self.apply_edit(&Edit::Transform { range, transform })?.count)
    }

    /// Fills empty cells of the range with the nearest value above them (`from_above`, which
    /// may come from the row just above the range) or with `value`.
    pub fn fill_empty(&mut self, range: CellRange, from_above: bool, value: &str) -> Result<usize> {
        Ok(self.apply_edit(&Edit::FillEmpty { range, from_above, value: value.to_owned() })?.count)
    }

    pub(crate) fn prepare_fill_empty(
        &self,
        range: CellRange,
        from_above: bool,
        value: &str,
        progress: &Progress,
    ) -> Result<Option<Prepared>> {
        self.check_range(&range)?;
        let columns: Vec<usize> = range.columns().collect();
        if !from_above {
            return Ok(self.cell_updates(range.rows(), &columns, progress, |_, _, current| {
                current.trim().is_empty().then(|| value.to_owned())
            }));
        }
        // Row by row: a value carries down to the empty cells below it.
        let mut above: Vec<Option<String>> = match range.top.checked_sub(1).and_then(|row| self.visible_record(row)) {
            Some(record) => columns.iter().map(|&column| Some(self.value_at(record, column).into_owned())).collect(),
            None => vec![None; columns.len()],
        };
        let rows = range.rows();
        progress.expect(rows.len());
        let mut updates = Vec::new();
        for (index, row) in rows.clone().enumerate() {
            if index % BLOCK == 0 {
                if progress.is_cancelled() {
                    return Ok(None);
                }
                progress.advance(BLOCK.min(rows.len() - index));
            }
            let Some(record) = self.visible_record(row) else { continue };
            let update = self.updated_record(record, columns.iter().copied(), |column, current| {
                let slot = &mut above[column - range.left];
                if current.trim().is_empty() {
                    slot.clone()
                } else {
                    *slot = Some(current.to_owned());
                    None
                }
            });
            updates.extend(update);
        }
        Ok(Some(Prepared::rows(updates, self.column_count(), None)))
    }

    /// Splits the values of `column` by `separator` (a regular expression when `regex`) into new
    /// columns inserted right after it; the column itself stays. With a header row the new
    /// columns are named after it: "Name 1", "Name 2"… Returns the number of new columns.
    pub fn split_column(&mut self, column: usize, separator: &str, regex: bool) -> Result<usize> {
        let edit = Edit::SplitColumn { column, separator: separator.to_owned(), regex };
        Ok(self.apply_edit(&edit)?.count)
    }

    pub(crate) fn prepare_split(
        &self,
        column: usize,
        separator: &str,
        regex: bool,
        progress: &Progress,
    ) -> Result<Option<Prepared>> {
        self.check_structure(SheetStructure::ColumnsFixed)?;
        self.check_column_index(column)?;
        if separator.is_empty() {
            return Err(Error::InvalidPattern("the separator is empty".into()));
        }
        let pattern = if regex {
            Some(Regex::new(separator).map_err(|error| Error::InvalidPattern(error.to_string()))?)
        } else {
            None
        };
        let split = |value: &str| -> Vec<String> {
            match &pattern {
                Some(pattern) => pattern.split(value).map(str::to_owned).collect(),
                None => value.split(separator).map(str::to_owned).collect(),
            }
        };

        // Two passes: the parts of every value, then the rows with as many new columns as the
        // longest split needs.
        let rows = 0..self.records.len();
        progress.expect(2 * rows.len());
        let first_data_row = self.header_offset();
        let Some(parts) = map_rows(rows.clone(), progress, |row| {
            Some((row >= first_data_row).then(|| split(&self.value_at(row, column))))
        }) else {
            return Ok(None);
        };
        let count = parts.iter().flatten().map(Vec::len).max().unwrap_or(0);
        if count <= 1 {
            return Ok(Some(Prepared::nothing()));
        }
        let header = (first_data_row == 1).then(|| self.value_at(0, column).into_owned());
        let names: Vec<String> = match &header {
            Some(name) => (1..=count).map(|n| format!("{name} {n}")).collect(),
            None => Vec::new(),
        };

        let records = map_rows(rows, progress, |row| {
            let record = &self.records[row];
            let mut fields = self.fields(record);
            if record.is_blank() || fields.len() <= column {
                return Some(record.clone());
            }
            let values = parts[row].as_ref().unwrap_or(&names);
            let new = (0..count).map(|n| Field::Text(values.get(n).cloned().unwrap_or_default()));
            fields.splice(column + 1..column + 1, new);
            Some(record.with_fields(fields))
        });
        drop_later(parts);
        let Some(records) = records else { return Ok(None) };
        Ok(Some(Prepared {
            change: Change::All { records, columns_after: self.column_count() + count },
            outcome: EditOutcome { changed: false, count, report: None },
        }))
    }

    /// The parts of the values of `column` split by `separator`, by row, for new columns
    /// right after it: their number and the values of each row, the header row's naming them
    /// "Name 1", "Name 2"…; `None` when no value splits.
    pub(crate) fn split_values(&self, column: usize, separator: &str, regex: bool) -> Result<Option<(usize, Vec<Vec<Field>>)>> {
        self.check_column_index(column)?;
        if separator.is_empty() {
            return Err(Error::InvalidPattern("the separator is empty".into()));
        }
        let pattern = if regex { Some(Regex::new(separator).map_err(|error| Error::InvalidPattern(error.to_string()))?) } else { None };
        let first_data_row = self.header_offset();
        let parts: Vec<Option<Vec<String>>> = (0..self.records.len())
            .map(|row| {
                (row >= first_data_row && !self.records[row].is_blank()).then(|| {
                    let value = self.value_at(row, column);
                    match &pattern {
                        Some(pattern) => pattern.split(&value).map(str::to_owned).collect(),
                        None => value.split(separator).map(str::to_owned).collect(),
                    }
                })
            })
            .collect();
        let count = parts.iter().flatten().map(Vec::len).max().unwrap_or(0);
        if count <= 1 {
            return Ok(None);
        }
        let header = (first_data_row == 1).then(|| self.value_at(0, column).into_owned());
        let values = parts
            .into_iter()
            .enumerate()
            .map(|(row, parts)| match (parts, &header) {
                (Some(parts), _) => parts.into_iter().map(Field::Text).collect(),
                (None, Some(name)) if row == 0 => (1..=count).map(|n| Field::Text(format!("{name} {n}"))).collect(),
                _ => Vec::new(),
            })
            .collect();
        Ok(Some((count, values)))
    }

    /// The non-empty values of `columns` joined with `separator`, by row, for a new column
    /// after the last of them: where it goes and its values; `None` without columns.
    pub(crate) fn merge_values(&self, columns: &[usize], separator: &str) -> Result<Option<(usize, Vec<Vec<Field>>)>> {
        let mut columns = columns.to_vec();
        columns.sort_unstable();
        columns.dedup();
        let Some(&last) = columns.last() else { return Ok(None) };
        self.check_column_index(last)?;
        let header_rows = self.header_offset();
        let values = (0..self.records.len())
            .map(|row| {
                let parts: Vec<String> =
                    columns.iter().map(|&column| self.value_at(row, column).into_owned()).filter(|value| !value.is_empty()).collect();
                let joiner = if row < header_rows && separator.trim().is_empty() { " " } else { separator };
                vec![Field::Text(parts.join(joiner))]
            })
            .collect();
        Ok(Some((last + 1, values)))
    }

    /// Joins the non-empty values of `columns` with `separator` into a new column inserted after
    /// the last of them; the columns themselves stay.
    pub fn merge_columns(&mut self, columns: &[usize], separator: &str) -> Result<()> {
        self.apply_edit(&Edit::MergeColumns { columns: columns.to_vec(), separator: separator.to_owned() })?;
        Ok(())
    }

    pub(crate) fn prepare_merge(&self, columns: &[usize], separator: &str, progress: &Progress) -> Result<Option<Prepared>> {
        self.check_structure(SheetStructure::ColumnsFixed)?;
        let mut columns = columns.to_vec();
        columns.sort_unstable();
        columns.dedup();
        let Some(&last) = columns.last() else { return Ok(Some(Prepared::nothing())) };
        self.check_column_index(last)?;
        let at = last + 1;
        let header_rows = self.header_offset();
        let rows = 0..self.records.len();
        progress.expect(rows.len());
        let Some(records) = map_rows(rows, progress, |row| {
            let record = &self.records[row];
            let values: Vec<String> = columns
                .iter()
                .map(|&column| self.value_at(row, column).into_owned())
                .filter(|value| !value.is_empty())
                .collect();
            let joiner = if row < header_rows && separator.trim().is_empty() { " " } else { separator };
            let merged = values.join(joiner);
            let mut fields = self.fields(record);
            if record.is_blank() || (merged.is_empty() && fields.len() <= at) {
                return Some(record.clone());
            }
            if fields.len() < at {
                fields.resize(at, Field::Text(String::new()));
            }
            fields.insert(at, Field::Text(merged));
            Some(record.with_fields(fields))
        }) else {
            return Ok(None);
        };
        Ok(Some(Prepared {
            change: Change::All { records, columns_after: self.column_count() + 1 },
            outcome: EditOutcome { changed: false, count: 1, report: None },
        }))
    }

    /// Deletes visible rows that repeat an earlier visible row, comparing `columns` (all columns
    /// when empty). With `normalize`, values are compared trimmed and case-insensitively.
    /// Returns the number of rows removed.
    pub fn remove_duplicates(&mut self, columns: &[usize], normalize: bool) -> Result<usize> {
        Ok(self.apply_edit(&Edit::RemoveDuplicates { columns: columns.to_vec(), normalize })?.count)
    }

    pub(crate) fn prepare_duplicates(&self, columns: &[usize], normalize: bool, progress: &Progress) -> Result<Option<Prepared>> {
        self.check_structure(SheetStructure::RowsFixed)?;
        for &column in columns {
            self.check_column_index(column)?;
        }
        let columns: Vec<usize> = if columns.is_empty() {
            (0..self.column_count()).collect()
        } else {
            columns.to_vec()
        };
        // Keys are made on all cores; the earliest row with a key stays.
        let rows = 0..self.visible_row_count();
        progress.expect(2 * rows.len());
        let Some(keys) = map_rows(rows, progress, |row| {
            let record = self.visible_record(row)?;
            let key: Vec<String> = columns
                .iter()
                .map(|&column| {
                    let value = self.value_at(record, column);
                    if normalize { value.trim().to_lowercase() } else { value.into_owned() }
                })
                .collect();
            Some((record, key))
        }) else {
            return Ok(None);
        };
        let mut seen = HashSet::with_capacity(keys.len());
        let mut duplicates = Vec::new();
        let total = keys.len();
        let mut keys = keys.into_iter();
        for start in (0..total).step_by(BLOCK) {
            if progress.is_cancelled() {
                drop_later((keys, seen));
                return Ok(None);
            }
            let count = BLOCK.min(total - start);
            for (record, key) in keys.by_ref().take(count) {
                if !seen.insert(key) {
                    duplicates.push(record);
                }
            }
            progress.advance(count);
        }
        drop_later(seen);
        Ok(Some(Prepared {
            outcome: EditOutcome { changed: false, count: duplicates.len(), report: None },
            change: Change::Remove(duplicates),
        }))
    }

    fn check_column_index(&self, column: usize) -> Result<()> {
        if column < self.column_count() {
            Ok(())
        } else {
            Err(Error::ColumnOutOfRange {
                column,
                columns: self.column_count(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms() {
        assert_eq!(TextTransform::Trim.apply("  a b  "), "a b");
        assert_eq!(TextTransform::CollapseSpaces.apply("  a \t  b\n c "), "a b c");
        assert_eq!(TextTransform::Uppercase.apply("ёлка tree"), "ЁЛКА TREE");
        assert_eq!(TextTransform::Lowercase.apply("МОСКВА"), "москва");
        assert_eq!(
            TextTransform::Capitalize.apply("иван ПЕТРОВ-водкин o'neil"),
            "Иван Петров-Водкин O'neil"
        );
    }
}

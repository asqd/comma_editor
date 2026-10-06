//! Column operations: text transforms, filling blanks, splitting and merging columns,
//! removing duplicate rows. Each is one undoable step.

use std::collections::HashSet;

use regex::Regex;

use crate::document::{Document, Field, Record};
use crate::error::{Error, Result};
use crate::range::CellRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        self.check_range(&range)?;
        Ok(self.edit_range(Vec::new(), range.rows(), range.columns(), false, |_, _, value| {
            Some(transform.apply(value))
        }))
    }

    /// Fills empty cells of the range with the nearest value above them (`from_above`, which
    /// may come from the row just above the range) or with `value`.
    pub fn fill_empty(&mut self, range: CellRange, from_above: bool, value: &str) -> Result<usize> {
        self.check_range(&range)?;
        let mut above: Vec<Option<String>> = match range.top.checked_sub(1).and_then(|row| self.visible_record(row)) {
            Some(record) if from_above => range
                .columns()
                .map(|column| Some(self.value_at(record, column).into_owned()))
                .collect(),
            _ => vec![None; range.columns().len()],
        };
        Ok(self.edit_range(Vec::new(), range.rows(), range.columns(), false, |_, column, current| {
            let slot = &mut above[column - range.left];
            if !current.trim().is_empty() {
                *slot = Some(current.to_owned());
                return None;
            }
            if from_above { slot.clone() } else { Some(value.to_owned()) }
        }))
    }

    /// Splits the values of `column` by `separator` (a regular expression when `regex`) into new
    /// columns inserted right after it; the column itself stays. With a header row the new
    /// columns are named after it: "Name 1", "Name 2"… Returns the number of new columns.
    pub fn split_column(&mut self, column: usize, separator: &str, regex: bool) -> Result<usize> {
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

        let first_data_row = self.header_offset();
        let parts: Vec<Option<Vec<String>>> = (0..self.records.len())
            .map(|row| (row >= first_data_row).then(|| split(&self.value_at(row, column))))
            .collect();
        let count = parts.iter().flatten().map(Vec::len).max().unwrap_or(0);
        if count <= 1 {
            return Ok(0);
        }
        let header = (first_data_row == 1).then(|| self.value_at(0, column).into_owned());
        let names: Vec<String> = match &header {
            Some(name) => (1..=count).map(|n| format!("{name} {n}")).collect(),
            None => Vec::new(),
        };

        let updated = self
            .records
            .iter()
            .zip(&parts)
            .map(|(record, parts)| {
                let mut fields = self.fields(record);
                if record.is_blank() || fields.len() <= column {
                    return record.clone();
                }
                let values = match parts {
                    Some(parts) => parts.clone(),
                    None => names.clone(),
                };
                let new = (0..count).map(|n| Field::Text(values.get(n).cloned().unwrap_or_default()));
                fields.splice(column + 1..column + 1, new);
                Record::modified(fields, record.term())
            })
            .collect();
        self.replace_records(updated, self.column_count() + count, false);
        self.reset_view();
        Ok(count)
    }

    /// Joins the non-empty values of `columns` with `separator` into a new column inserted after
    /// the last of them; the columns themselves stay.
    pub fn merge_columns(&mut self, columns: &[usize], separator: &str) -> Result<()> {
        let mut columns = columns.to_vec();
        columns.sort_unstable();
        columns.dedup();
        let Some(&last) = columns.last() else { return Ok(()) };
        self.check_column_index(last)?;
        let at = last + 1;
        let header_rows = self.header_offset();
        let updated = (0..self.records.len())
            .map(|row| {
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
                    return record.clone();
                }
                if fields.len() < at {
                    fields.resize(at, Field::Text(String::new()));
                }
                fields.insert(at, Field::Text(merged));
                Record::modified(fields, record.term())
            })
            .collect();
        self.replace_records(updated, self.column_count() + 1, false);
        self.reset_view();
        Ok(())
    }

    /// Deletes visible rows that repeat an earlier visible row, comparing `columns` (all columns
    /// when empty). With `normalize`, values are compared trimmed and case-insensitively.
    /// Returns the number of rows removed.
    pub fn remove_duplicates(&mut self, columns: &[usize], normalize: bool) -> Result<usize> {
        for &column in columns {
            self.check_column_index(column)?;
        }
        let columns: Vec<usize> = if columns.is_empty() {
            (0..self.column_count()).collect()
        } else {
            columns.to_vec()
        };
        let mut seen = HashSet::new();
        let mut duplicates = Vec::new();
        for row in 0..self.visible_row_count() {
            let Some(record) = self.visible_record(row) else { continue };
            let key: Vec<String> = columns
                .iter()
                .map(|&column| {
                    let value = self.value_at(record, column);
                    if normalize { value.trim().to_lowercase() } else { value.into_owned() }
                })
                .collect();
            if !seen.insert(key) {
                duplicates.push(record);
            }
        }
        if !duplicates.is_empty() {
            self.delete_rows(&duplicates)?;
        }
        Ok(duplicates.len())
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

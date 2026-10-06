use std::path::Path;

use crate::dialect::{self, Dialect};
use crate::encoding::{self, Prepared, TextEncoding};
use crate::error::{Error, Result};
use crate::history::{History, Splice, Transaction};
use crate::parser::{self, Terminator};

#[derive(Clone, Debug)]
pub(crate) enum Record {
    /// Untouched record: a byte range of the original file.
    Original {
        start: usize,
        end: usize,
        term: Terminator,
    },
    /// Edited or inserted record.
    Modified(Box<Modified>),
}

#[derive(Clone, Debug)]
pub(crate) struct Modified {
    pub fields: Vec<Field>,
    pub term: Terminator,
}

#[derive(Clone, Debug)]
pub(crate) enum Field {
    /// Untouched field: raw bytes of the original file, quotes included.
    Raw { start: usize, end: usize },
    Text(String),
}

impl Record {
    pub(crate) fn term(&self) -> Terminator {
        match self {
            Self::Original { term, .. } => *term,
            Self::Modified(modified) => modified.term,
        }
    }

    fn is_blank(&self) -> bool {
        matches!(self, Self::Original { start, end, .. } if start == end)
    }

    fn modified(fields: Vec<Field>, term: Terminator) -> Self {
        Self::Modified(Box::new(Modified { fields, term }))
    }
}

/// Summary of a document for the UI.
#[derive(Clone, Debug)]
pub struct Info {
    pub encoding: TextEncoding,
    pub has_bom: bool,
    pub dialect: Dialect,
    pub has_header: bool,
    pub trailing_newline: bool,
    pub row_count: usize,
    pub column_count: usize,
    pub unterminated_quote: bool,
}

/// A CSV file being edited. Rows include the header row, if there is one.
pub struct Document {
    pub(crate) buf: Vec<u8>,
    pub(crate) content_start: usize,
    pub(crate) encoding: TextEncoding,
    pub(crate) has_bom: bool,
    pub(crate) dialect: Dialect,
    pub(crate) trailing_newline: bool,
    pub(crate) records: Vec<Record>,
    has_header: bool,
    column_count: usize,
    unterminated_quote: bool,
    history: History,
}

impl Document {
    /// A new untitled document: one empty row of three columns, UTF-8, comma-separated.
    pub fn new_empty() -> Self {
        let columns = 3;
        Self {
            buf: Vec::new(),
            content_start: 0,
            encoding: TextEncoding::Utf8,
            has_bom: false,
            dialect: Dialect::default(),
            trailing_newline: true,
            records: vec![Record::modified(
                vec![Field::Text(String::new()); columns],
                Terminator::Lf,
            )],
            has_header: false,
            column_count: columns,
            unterminated_quote: false,
            history: History::default(),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::from_bytes(std::fs::read(path)?))
    }

    /// Detects the encoding and dialect, then indexes the records.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let prepared = encoding::prepare(bytes);
        let dialect = dialect::sniff(&prepared.buffer, prepared.content_start);
        Self::parse(prepared, dialect)
    }

    fn parse(prepared: Prepared, dialect: Dialect) -> Self {
        let scan = parser::scan(
            &prepared.buffer,
            prepared.content_start,
            dialect.delimiter,
            dialect.quote,
            None,
        );
        let trailing_newline = scan
            .records
            .last()
            .is_some_and(|record| record.term != Terminator::None);
        let records = scan
            .records
            .into_iter()
            .map(|r| Record::Original {
                start: r.start,
                end: r.end,
                term: r.term,
            })
            .collect();
        let mut document = Self {
            buf: prepared.buffer,
            content_start: prepared.content_start,
            encoding: prepared.encoding,
            has_bom: prepared.has_bom,
            dialect,
            trailing_newline,
            records,
            has_header: false,
            column_count: scan.max_fields,
            unterminated_quote: scan.unterminated_quote,
            history: History::default(),
        };
        document.has_header = document.records.len() > 1
            && document
                .row(0)
                .is_ok_and(|row| dialect::sniff_header(&row));
        document
    }

    pub fn info(&self) -> Info {
        Info {
            encoding: self.encoding,
            has_bom: self.has_bom,
            dialect: self.dialect,
            has_header: self.has_header,
            trailing_newline: self.trailing_newline,
            row_count: self.records.len(),
            column_count: self.column_count,
            unterminated_quote: self.unterminated_quote,
        }
    }

    pub fn row_count(&self) -> usize {
        self.records.len()
    }

    pub fn column_count(&self) -> usize {
        self.column_count
    }

    pub fn has_header(&self) -> bool {
        self.has_header
    }

    /// Whether the first row is shown as column names. Does not change the file.
    pub fn set_has_header(&mut self, has_header: bool) {
        self.has_header = has_header;
    }

    /// Values of one row. Rows can be shorter than `column_count`.
    pub fn row(&self, index: usize) -> Result<Vec<String>> {
        let record = self.records.get(index).ok_or(Error::RowOutOfRange {
            row: index,
            rows: self.records.len(),
        })?;
        Ok(match record {
            Record::Original { start, end, .. } => self
                .raw_fields(*start, *end)
                .map(|(s, e)| self.raw_value(s, e))
                .collect(),
            Record::Modified(modified) => modified
                .fields
                .iter()
                .map(|field| self.field_value(field))
                .collect(),
        })
    }

    pub fn cell(&self, row: usize, column: usize) -> Result<String> {
        Ok(self.row(row)?.into_iter().nth(column).unwrap_or_default())
    }

    /// Sets one cell. Returns `false` and records nothing if the cell already holds `value`.
    pub fn set_cell(&mut self, row: usize, column: usize, value: &str) -> Result<bool> {
        self.check_row(row)?;
        self.check_column(column)?;
        let record = &self.records[row];
        let mut fields = self.fields(record);
        let current = fields.get(column).map(|f| self.field_value(f));
        if current.as_deref().unwrap_or_default() == value {
            return Ok(false);
        }
        if fields.len() <= column {
            fields.resize(column + 1, Field::Text(String::new()));
        }
        fields[column] = Field::Text(value.to_owned());
        let updated = Record::modified(fields, record.term());
        let splice = Splice::apply(&mut self.records, row, 1, vec![updated]);
        self.commit(vec![splice], self.column_count);
        Ok(true)
    }

    /// Inserts `count` empty rows before row `at` (`at == row_count` appends).
    pub fn insert_rows(&mut self, at: usize, count: usize) -> Result<()> {
        if at > self.records.len() {
            return Err(Error::RowOutOfRange {
                row: at,
                rows: self.records.len(),
            });
        }
        if count == 0 {
            return Ok(());
        }
        let columns = self.column_count.max(1);
        let term = self.dialect.line_ending.terminator();
        let rows = (0..count)
            .map(|_| Record::modified(vec![Field::Text(String::new()); columns], term))
            .collect();
        let splice = Splice::apply(&mut self.records, at, 0, rows);
        self.commit(vec![splice], columns);
        Ok(())
    }

    /// Deletes the given rows as one undoable step.
    pub fn delete_rows(&mut self, rows: &[usize]) -> Result<()> {
        let mut rows = rows.to_vec();
        rows.sort_unstable();
        rows.dedup();
        match rows.last() {
            None => return Ok(()),
            Some(&last) => self.check_row(last)?,
        }
        // Contiguous runs, from the end so that earlier indices stay valid.
        let mut splices = Vec::new();
        let mut end = rows.len();
        while end > 0 {
            let mut start = end - 1;
            while start > 0 && rows[start - 1] + 1 == rows[start] {
                start -= 1;
            }
            splices.push(Splice::apply(
                &mut self.records,
                rows[start],
                end - start,
                Vec::new(),
            ));
            end = start;
        }
        self.commit(splices, self.column_count);
        Ok(())
    }

    /// Inserts an empty column before column `at` (`at == column_count` appends). Only rows that
    /// continue past `at` change; when appending, full-width rows get a trailing empty field.
    /// Short rows and blank lines stay untouched.
    pub fn insert_column(&mut self, at: usize) -> Result<()> {
        if at > self.column_count {
            return Err(Error::ColumnOutOfRange {
                column: at,
                columns: self.column_count,
            });
        }
        let appending = at == self.column_count;
        let updated = self
            .records
            .iter()
            .map(|record| {
                let mut fields = self.fields(record);
                let affected = fields.len() > at || (appending && fields.len() == at);
                if record.is_blank() || !affected {
                    return record.clone();
                }
                fields.insert(at, Field::Text(String::new()));
                Record::modified(fields, record.term())
            })
            .collect();
        self.replace_all(updated, self.column_count + 1);
        Ok(())
    }

    pub fn delete_column(&mut self, at: usize) -> Result<()> {
        self.check_column(at)?;
        if self.column_count <= 1 {
            return Err(Error::LastColumn);
        }
        let updated = self
            .records
            .iter()
            .map(|record| {
                let mut fields = self.fields(record);
                if record.is_blank() || fields.len() <= at {
                    return record.clone();
                }
                fields.remove(at);
                Record::modified(fields, record.term())
            })
            .collect();
        self.replace_all(updated, self.column_count - 1);
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) -> bool {
        self.history
            .undo(&mut self.records)
            .map(|columns| self.column_count = columns)
            .is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.history
            .redo(&mut self.records)
            .map(|columns| self.column_count = columns)
            .is_some()
    }

    /// Encoding of `buf`: UTF-16 files are held as UTF-8.
    pub(crate) fn buffer_encoding(&self) -> TextEncoding {
        if self.encoding.is_utf16() {
            TextEncoding::Utf8
        } else {
            self.encoding
        }
    }

    fn commit(&mut self, splices: Vec<Splice>, columns_after: usize) {
        self.history.record(Transaction {
            splices,
            columns_before: self.column_count,
            columns_after,
        });
        self.column_count = columns_after;
    }

    fn replace_all(&mut self, records: Vec<Record>, columns_after: usize) {
        let len = self.records.len();
        let splice = Splice::apply(&mut self.records, 0, len, records);
        self.commit(vec![splice], columns_after);
    }

    fn check_row(&self, row: usize) -> Result<()> {
        if row < self.records.len() {
            Ok(())
        } else {
            Err(Error::RowOutOfRange {
                row,
                rows: self.records.len(),
            })
        }
    }

    fn check_column(&self, column: usize) -> Result<()> {
        if column < self.column_count {
            Ok(())
        } else {
            Err(Error::ColumnOutOfRange {
                column,
                columns: self.column_count,
            })
        }
    }

    fn raw_fields(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> {
        parser::split_fields(&self.buf[start..end], self.dialect.delimiter, self.dialect.quote)
            .into_iter()
            .map(move |(s, e)| (start + s, start + e))
    }

    fn fields(&self, record: &Record) -> Vec<Field> {
        match record {
            Record::Original { start, end, .. } => self
                .raw_fields(*start, *end)
                .map(|(start, end)| Field::Raw { start, end })
                .collect(),
            Record::Modified(modified) => modified.fields.clone(),
        }
    }

    fn field_value(&self, field: &Field) -> String {
        match field {
            Field::Raw { start, end } => self.raw_value(*start, *end),
            Field::Text(text) => text.clone(),
        }
    }

    fn raw_value(&self, start: usize, end: usize) -> String {
        let value = parser::unquote(&self.buf[start..end], self.dialect.quote);
        self.buffer_encoding().decode(&value).into_owned()
    }
}

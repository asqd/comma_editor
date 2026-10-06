use std::borrow::Cow;
use std::path::Path;

use crate::dialect::{self, Dialect};
use crate::encoding::{self, Prepared, TextEncoding};
use crate::error::{Error, Result};
use crate::history::{History, Splice, Transaction};
use crate::parser::{self, Terminator};
use crate::view::{self, CompiledFilter, ViewSpec};

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

    pub(crate) fn modified(fields: Vec<Field>, term: Terminator) -> Self {
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
    /// Rows shown by the current view: data rows that pass the filters.
    pub visible_row_count: usize,
    pub unterminated_quote: bool,
}

/// Sorting and filtering of the data rows. `rows` holds the record indices to show, in order;
/// `None` means every data row in file order.
#[derive(Default)]
struct View {
    spec: ViewSpec,
    filters: Vec<CompiledFilter>,
    rows: Option<Vec<usize>>,
}

/// A CSV file being edited.
///
/// *Rows* are records of the file, the header row included. *Visible rows* are the data rows
/// shown by the current view: without the header, filtered and sorted.
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
    view: View,
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
            view: View::default(),
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
            view: View::default(),
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
            visible_row_count: self.visible_row_count(),
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

    /// Whether the first row holds column names. Does not change the file.
    pub fn set_has_header(&mut self, has_header: bool) {
        self.has_header = has_header;
        self.refresh_view();
    }

    /// Values of one row. Rows can be shorter than `column_count`.
    pub fn row(&self, index: usize) -> Result<Vec<String>> {
        self.check_row(index)?;
        Ok(self.values(index).into_iter().map(Cow::into_owned).collect())
    }

    pub fn cell(&self, row: usize, column: usize) -> Result<String> {
        self.check_row(row)?;
        Ok(self.value_at(row, column).into_owned())
    }

    // MARK: View

    pub fn view_spec(&self) -> &ViewSpec {
        &self.view.spec
    }

    /// Sorts and filters the visible rows. Fails on an invalid regular expression or column.
    pub fn set_view(&mut self, spec: ViewSpec) -> Result<()> {
        let columns = spec
            .sort
            .iter()
            .map(|key| Some(key.column))
            .chain(spec.filters.iter().map(|filter| filter.column));
        for column in columns.flatten() {
            self.check_column(column)?;
        }
        let filters = spec
            .filters
            .iter()
            .map(CompiledFilter::new)
            .collect::<Result<Vec<_>>>()?;
        self.view = View {
            spec,
            filters,
            rows: None,
        };
        self.refresh_view();
        Ok(())
    }

    pub fn visible_row_count(&self) -> usize {
        match &self.view.rows {
            Some(rows) => rows.len(),
            None => self.records.len() - self.header_offset(),
        }
    }

    /// Row index of a visible row.
    pub fn visible_record(&self, visible_row: usize) -> Option<usize> {
        match &self.view.rows {
            Some(rows) => rows.get(visible_row).copied(),
            None => {
                let row = visible_row + self.header_offset();
                (row < self.records.len()).then_some(row)
            }
        }
    }

    /// Where row `row` of the file is shown, if it is visible.
    pub fn visible_index(&self, row: usize) -> Option<usize> {
        match &self.view.rows {
            Some(rows) => rows.iter().position(|&visible| visible == row),
            None => row
                .checked_sub(self.header_offset())
                .filter(|_| row < self.records.len()),
        }
    }

    pub fn visible_row(&self, visible_row: usize) -> Result<Vec<String>> {
        let row = self.visible_record_checked(visible_row)?;
        self.row(row)
    }

    /// Row index where rows inserted before `visible_row` go; past the last visible row,
    /// right after it.
    pub fn insertion_row(&self, visible_row: usize) -> usize {
        let visible = self.visible_row_count();
        let anchor = if visible_row < visible {
            self.visible_record(visible_row)
        } else {
            visible.checked_sub(1).and_then(|last| self.visible_record(last)).map(|row| row + 1)
        };
        anchor.unwrap_or(self.records.len())
    }

    /// Makes the current sort order permanent: reorders the rows of the file as one undoable
    /// step and clears the sort. Only without filters, so no row is left out.
    /// Returns `false` if there is nothing to apply.
    pub fn apply_sort(&mut self) -> Result<bool> {
        if self.view.spec.sort.is_empty() || !self.view.spec.filters.is_empty() {
            return Ok(false);
        }
        let start = self.header_offset();
        let order = self.view.rows.take().unwrap_or_default();
        self.view.spec.sort.clear();
        if order.iter().enumerate().all(|(index, &row)| row == start + index) {
            self.refresh_view();
            return Ok(false);
        }
        let mut reordered: Vec<Record> = self.records[..start].to_vec();
        reordered.extend(order.iter().map(|&row| self.records[row].clone()));
        self.replace_records(reordered, self.column_count, true);
        self.refresh_view();
        Ok(true)
    }

    fn refresh_view(&mut self) {
        if self.view.spec.is_empty() {
            self.view.rows = None;
            return;
        }
        let filters = &self.view.filters;
        let mut rows: Vec<usize> = (self.header_offset()..self.records.len())
            .filter(|&row| self.passes(row, filters))
            .collect();
        if !self.view.spec.sort.is_empty() {
            view::sort_rows(&mut rows, &self.view.spec.sort, |row, column| {
                self.value_at(row, column)
            });
        }
        self.view.rows = Some(rows);
    }

    fn passes(&self, row: usize, filters: &[CompiledFilter]) -> bool {
        filters.iter().all(|filter| {
            let hit = match filter.column() {
                Some(column) => filter.test(&self.value_at(row, column)),
                None => self.values(row).iter().any(|value| filter.test(value)),
            };
            hit != filter.negated()
        })
    }

    /// Columns moved, so sort keys and filters no longer point at the right data.
    fn reset_view(&mut self) {
        self.view = View::default();
    }

    fn header_offset(&self) -> usize {
        usize::from(self.has_header && !self.records.is_empty())
    }

    pub(crate) fn visible_record_checked(&self, visible_row: usize) -> Result<usize> {
        self.visible_record(visible_row).ok_or(Error::RowOutOfRange {
            row: visible_row,
            rows: self.visible_row_count(),
        })
    }

    // MARK: Editing

    /// Sets one cell. Returns `false` and records nothing if the cell already holds `value`.
    /// The view keeps its order: an edited row stays where it is until the view is refreshed.
    pub fn set_cell(&mut self, row: usize, column: usize, value: &str) -> Result<bool> {
        self.check_row(row)?;
        self.check_column(column)?;
        if self.value_at(row, column) == value {
            return Ok(false);
        }
        let record = &self.records[row];
        let mut fields = self.fields(record);
        if fields.len() <= column {
            fields.resize(column + 1, Field::Text(String::new()));
        }
        fields[column] = Field::Text(value.to_owned());
        let updated = Record::modified(fields, record.term());
        let splice = Splice::apply(&mut self.records, row, 1, vec![updated]);
        self.commit(vec![splice], self.column_count, false);
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
        self.commit(vec![splice], columns, true);
        self.refresh_view();
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
        self.commit(splices, self.column_count, true);
        self.refresh_view();
        Ok(())
    }

    /// Inserts an empty column before column `at` (`at == column_count` appends). Only rows that
    /// continue past `at` change; when appending, full-width rows get a trailing empty field.
    /// Short rows and blank lines stay untouched. Resets the view.
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
        self.replace_records(updated, self.column_count + 1, false);
        self.reset_view();
        Ok(())
    }

    /// Deletes column `at`. Resets the view.
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
        self.replace_records(updated, self.column_count - 1, false);
        self.reset_view();
        Ok(())
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) -> bool {
        match self.history.undo(&mut self.records) {
            Some(reverted) => {
                self.after_history(reverted.columns, reverted.structural);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.history.redo(&mut self.records) {
            Some(reverted) => {
                self.after_history(reverted.columns, reverted.structural);
                true
            }
            None => false,
        }
    }

    fn after_history(&mut self, columns: usize, structural: bool) {
        if columns != self.column_count {
            self.column_count = columns;
            self.reset_view();
        } else if structural {
            self.refresh_view();
        }
    }

    pub(crate) fn commit(&mut self, splices: Vec<Splice>, columns_after: usize, structural: bool) {
        self.history.record(Transaction {
            splices,
            columns_before: self.column_count,
            columns_after,
            structural,
        });
        self.column_count = columns_after;
    }

    fn replace_records(&mut self, records: Vec<Record>, columns_after: usize, structural: bool) {
        let len = self.records.len();
        let splice = Splice::apply(&mut self.records, 0, len, records);
        self.commit(vec![splice], columns_after, structural);
    }

    // MARK: Values

    /// Encoding of `buf`: UTF-16 files are held as UTF-8.
    pub(crate) fn buffer_encoding(&self) -> TextEncoding {
        if self.encoding.is_utf16() {
            TextEncoding::Utf8
        } else {
            self.encoding
        }
    }

    /// One value; empty past the end of a short row. `row` must exist.
    pub(crate) fn value_at(&self, row: usize, column: usize) -> Cow<'_, str> {
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        match &self.records[row] {
            Record::Original { start, end, .. } => {
                match parser::nth_field(&self.buf[*start..*end], delimiter, quote, column) {
                    Some((s, e)) => self.raw_value(start + s, start + e),
                    None => Cow::Borrowed(""),
                }
            }
            Record::Modified(modified) => match modified.fields.get(column) {
                Some(field) => self.field_value(field),
                None => Cow::Borrowed(""),
            },
        }
    }

    /// All values of a row. `row` must exist.
    pub(crate) fn values(&self, row: usize) -> Vec<Cow<'_, str>> {
        match &self.records[row] {
            Record::Original { start, end, .. } => self
                .raw_fields(*start, *end)
                .map(|(s, e)| self.raw_value(s, e))
                .collect(),
            Record::Modified(modified) => modified
                .fields
                .iter()
                .map(|field| self.field_value(field))
                .collect(),
        }
    }

    pub(crate) fn fields(&self, record: &Record) -> Vec<Field> {
        match record {
            Record::Original { start, end, .. } => self
                .raw_fields(*start, *end)
                .map(|(start, end)| Field::Raw { start, end })
                .collect(),
            Record::Modified(modified) => modified.fields.clone(),
        }
    }

    fn raw_fields(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> {
        parser::split_fields(&self.buf[start..end], self.dialect.delimiter, self.dialect.quote)
            .into_iter()
            .map(move |(s, e)| (start + s, start + e))
    }

    fn field_value<'a>(&'a self, field: &'a Field) -> Cow<'a, str> {
        match field {
            Field::Raw { start, end } => self.raw_value(*start, *end),
            Field::Text(text) => Cow::Borrowed(text),
        }
    }

    fn raw_value(&self, start: usize, end: usize) -> Cow<'_, str> {
        let encoding = self.buffer_encoding();
        match parser::unquote(&self.buf[start..end], self.dialect.quote) {
            Cow::Borrowed(bytes) => encoding.decode(bytes),
            Cow::Owned(bytes) => Cow::Owned(encoding.decode(&bytes).into_owned()),
        }
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
}

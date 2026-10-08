use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::bytes::{self, Bytes};

use crate::dialect::{self, Dialect};
use crate::encoding::{self, Prepared, TextEncoding};
use crate::error::{Error, Result};
use crate::history::{History, Splice, Transaction};
use crate::parser::{self, Terminator};
use crate::view::{self, CompiledFilter, ViewSpec};
use crate::writer::SaveFormat;

#[derive(Clone, Debug)]
pub(crate) enum Record {
    /// Untouched record: `len` bytes of the original file from `start`. Kept to 16 bytes, since
    /// a large file has tens of millions of them.
    Original { start: usize, len: u32, term: Terminator },
    /// Untouched record longer than 4 GB, which only a quote left open in a huge file makes.
    Long(Box<(usize, usize, Terminator)>),
    /// Edited or inserted record.
    Modified(Box<Modified>),
}

/// A record as the rest of the code reads it, whatever way it is stored.
pub(crate) enum RecordRef<'a> {
    Original { start: usize, end: usize, term: Terminator },
    Modified(&'a Modified),
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
    /// The untouched record at bytes `start..end` of the file.
    pub(crate) fn original(start: usize, end: usize, term: Terminator) -> Self {
        match u32::try_from(end - start) {
            Ok(len) => Self::Original { start, len, term },
            Err(_) => Self::Long(Box::new((start, end, term))),
        }
    }

    pub(crate) fn get(&self) -> RecordRef<'_> {
        match self {
            Self::Original { start, len, term } => RecordRef::Original {
                start: *start,
                end: start + *len as usize,
                term: *term,
            },
            Self::Long(span) => RecordRef::Original { start: span.0, end: span.1, term: span.2 },
            Self::Modified(modified) => RecordRef::Modified(modified),
        }
    }

    pub(crate) fn term(&self) -> Terminator {
        match self.get() {
            RecordRef::Original { term, .. } => term,
            RecordRef::Modified(modified) => modified.term,
        }
    }

    pub(crate) fn is_blank(&self) -> bool {
        matches!(self, Self::Original { len: 0, .. })
    }

    pub(crate) fn modified(fields: Vec<Field>, term: Terminator) -> Self {
        Self::Modified(Box::new(Modified { fields, term }))
    }

    /// Bytes the record holds besides itself.
    pub(crate) fn heap_size(&self) -> usize {
        match self {
            Self::Original { .. } => 0,
            Self::Long(_) => size_of::<(usize, usize, Terminator)>(),
            Self::Modified(modified) => {
                let texts: usize = modified
                    .fields
                    .iter()
                    .map(|field| match field {
                        Field::Text(text) => text.capacity(),
                        Field::Raw { .. } => 0,
                    })
                    .sum();
                size_of::<Modified>() + modified.fields.capacity() * size_of::<Field>() + texts
            }
        }
    }
}

/// Summary of a document for the UI. Encoding, BOM, delimiter and line endings are those the
/// document will be saved with.
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
    /// The encoding and delimiter were detected with certainty, or chosen when opening.
    pub detection_confident: bool,
}

/// What to assume when opening a file instead of detecting it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenOptions {
    pub encoding: Option<TextEncoding>,
    pub delimiter: Option<u8>,
}

/// The first rows of a file as they would open with some options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePreview {
    pub encoding: TextEncoding,
    pub delimiter: u8,
    pub has_header: bool,
    pub rows: Vec<Vec<String>>,
    pub column_count: usize,
    pub detection_confident: bool,
}

/// Sorting and filtering of the data rows. `rows` holds the record indices to show, in order;
/// `None` means every data row in file order.
#[derive(Clone, Default)]
struct View {
    spec: ViewSpec,
    filters: Vec<CompiledFilter>,
    /// Shared with snapshots, which work off the document without copying it.
    rows: Option<Arc<Vec<usize>>>,
}

/// From this many separate runs of deleted rows on, the rows are removed in one pass over the
/// list rather than run by run, each of which moves the rows after it.
const SCATTERED_RUNS: usize = 16;

/// A CSV file being edited.
///
/// *Rows* are records of the file, the header row included. *Visible rows* are the data rows
/// shown by the current view: without the header, filtered and sorted.
pub struct Document {
    /// The file's bytes, shared with background work such as sorting.
    pub(crate) buf: Arc<Bytes>,
    pub(crate) content_start: usize,
    pub(crate) encoding: TextEncoding,
    pub(crate) has_bom: bool,
    pub(crate) dialect: Dialect,
    pub(crate) trailing_newline: bool,
    /// Shared with snapshots; copied only when changed while a snapshot is alive.
    pub(crate) records: Arc<Vec<Record>>,
    has_header: bool,
    column_count: usize,
    unterminated_quote: bool,
    detection_confident: bool,
    /// Counts changes to the rows, so work done on a snapshot knows whether it is still current.
    pub(crate) generation: u64,
    /// Rows past the first part of a large file are still being indexed; see `open_partially`.
    indexing: bool,
    history: History,
    view: View,
    /// Format to save in; the detected one unless "Save As" chose another.
    format: SaveFormat,
}

impl Document {
    /// A new untitled document: one empty row of three columns, UTF-8, comma-separated.
    pub fn new_empty() -> Self {
        let columns = 3;
        Self {
            buf: Arc::new(Bytes::Owned(Vec::new())),
            content_start: 0,
            encoding: TextEncoding::Utf8,
            has_bom: false,
            dialect: Dialect::default(),
            trailing_newline: true,
            records: Arc::new(vec![Record::modified(
                vec![Field::Text(String::new()); columns],
                Terminator::Lf,
            )]),
            has_header: false,
            column_count: columns,
            unterminated_quote: false,
            detection_confident: true,
            generation: 0,
            indexing: false,
            history: History::default(),
            view: View::default(),
            format: SaveFormat {
                encoding: TextEncoding::Utf8,
                bom: false,
                delimiter: b',',
                line_ending: crate::dialect::LineEnding::Lf,
            },
        }
    }

    /// Opens a file; large files are mapped from disk rather than read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, &OpenOptions::default())
    }

    pub fn open_with(path: impl AsRef<Path>, options: &OpenOptions) -> Result<Self> {
        Ok(Self::from_buffer(bytes::read_file(path.as_ref())?, options))
    }

    /// Detects the encoding and dialect, then indexes the records.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes_with(bytes, &OpenOptions::default())
    }

    /// Like `from_bytes`, with the encoding or delimiter given instead of detected.
    pub fn from_bytes_with(bytes: Vec<u8>, options: &OpenOptions) -> Self {
        Self::from_buffer(bytes.into(), options)
    }

    fn from_buffer(bytes: Bytes, options: &OpenOptions) -> Self {
        Self::from_buffer_partially(bytes, options, usize::MAX).0
    }

    /// Opens a file with only the rows in about its first `first_bytes` bytes indexed, so a
    /// large file shows at once. The rest is indexed with the returned `Indexer` and added with
    /// `append_index`; until then the document must not be edited or saved.
    pub fn open_partially(
        path: impl AsRef<Path>,
        options: &OpenOptions,
        first_bytes: usize,
    ) -> Result<(Self, Option<Indexer>)> {
        Ok(Self::from_buffer_partially(bytes::read_file(path.as_ref())?, options, first_bytes))
    }

    fn from_buffer_partially(bytes: Bytes, options: &OpenOptions, first_bytes: usize) -> (Self, Option<Indexer>) {
        let prepared = match options.encoding {
            Some(encoding) => encoding::prepare_as(bytes, encoding),
            None => encoding::prepare(bytes),
        };
        let (dialect, dialect_confident) =
            dialect::sniff_with(&prepared.buffer, prepared.content_start, options.delimiter);
        let confident = prepared.confident && dialect_confident;
        let stop = prepared.content_start.saturating_add(first_bytes);
        let (mut document, next) = Self::parse(prepared, dialect, stop);
        document.detection_confident = confident;
        let indexer = (next < document.buf.len()).then(|| {
            document.indexing = true;
            Indexer {
                buf: Arc::clone(&document.buf),
                position: next,
                delimiter: dialect.delimiter,
                quote: dialect.quote,
            }
        });
        (document, indexer)
    }

    /// Whether rows of a large file are still being indexed.
    pub fn is_indexing(&self) -> bool {
        self.indexing
    }

    /// Adds rows indexed in the background.
    pub fn append_index(&mut self, chunk: IndexChunk) {
        self.column_count = self.column_count.max(chunk.max_fields);
        self.unterminated_quote |= chunk.unterminated_quote;
        Arc::make_mut(&mut self.records).extend(chunk.records);
        if chunk.done {
            self.indexing = false;
            self.trailing_newline = self.records.last().is_some_and(|record| record.term() != Terminator::None);
        }
        self.generation += 1;
    }

    /// How the start of a file opens with `options`: up to `rows` rows read from at most the
    /// first `bytes` bytes, cut at a line break.
    pub fn preview_file(path: impl AsRef<Path>, options: &OpenOptions, rows: usize, bytes: usize) -> Result<FilePreview> {
        use std::io::Read;
        let mut data = Vec::new();
        std::fs::File::open(path)?.take(bytes as u64).read_to_end(&mut data)?;
        if data.len() == bytes
            && let Some(end) = data.iter().rposition(|&b| b == b'\n')
        {
            data.truncate(end + 1);
        }
        let document = Self::from_bytes_with(data, options);
        let rows: Vec<Vec<String>> = (0..document.row_count().min(rows))
            .filter_map(|row| document.row(row).ok())
            .collect();
        Ok(FilePreview {
            encoding: document.encoding,
            delimiter: document.dialect.delimiter,
            has_header: document.has_header,
            column_count: rows.iter().map(Vec::len).max().unwrap_or(0),
            rows,
            detection_confident: document.detection_confident,
        })
    }

    /// Indexes the records that start before `stop`; returns where indexing stopped.
    fn parse(prepared: Prepared, dialect: Dialect, stop: usize) -> (Self, usize) {
        let scan = parser::scan_until(
            &prepared.buffer,
            prepared.content_start,
            dialect.delimiter,
            dialect.quote,
            stop,
        );
        let next = scan.next;
        let trailing_newline = scan
            .records
            .last()
            .is_some_and(|record| record.term != Terminator::None);
        let records: Vec<Record> = scan
            .records
            .into_iter()
            .map(|r| Record::original(r.start, r.end, r.term))
            .collect();
        let records = Arc::new(records);
        let mut document = Self {
            buf: Arc::new(prepared.buffer),
            content_start: prepared.content_start,
            encoding: prepared.encoding,
            has_bom: prepared.has_bom,
            dialect,
            trailing_newline,
            records,
            has_header: false,
            column_count: scan.max_fields,
            unterminated_quote: scan.unterminated_quote,
            detection_confident: true,
            generation: 0,
            indexing: false,
            history: History::default(),
            view: View::default(),
            format: SaveFormat {
                encoding: prepared.encoding,
                bom: prepared.has_bom,
                delimiter: dialect.delimiter,
                line_ending: dialect.line_ending,
            },
        };
        document.has_header = document.records.len() > 1
            && document
                .row(0)
                .is_ok_and(|row| dialect::sniff_header(&row));
        (document, next)
    }

    pub fn info(&self) -> Info {
        Info {
            encoding: self.format.encoding,
            has_bom: self.format.bom,
            dialect: Dialect {
                delimiter: self.format.delimiter,
                line_ending: self.format.line_ending,
                ..self.dialect
            },
            has_header: self.has_header,
            trailing_newline: self.trailing_newline,
            row_count: self.records.len(),
            column_count: self.column_count,
            visible_row_count: self.visible_row_count(),
            unterminated_quote: self.unterminated_quote,
            detection_confident: self.detection_confident,
        }
    }

    /// The format the file had when it was opened.
    pub fn source_format(&self) -> SaveFormat {
        SaveFormat {
            encoding: self.encoding,
            bom: self.has_bom,
            delimiter: self.dialect.delimiter,
            line_ending: self.dialect.line_ending,
        }
    }

    pub fn save_format(&self) -> SaveFormat {
        self.format
    }

    /// Format for later saves, as chosen in "Save As". The original format keeps saves byte-exact.
    pub fn set_save_format(&mut self, format: SaveFormat) -> Result<()> {
        format.validate(self.dialect.quote)?;
        self.format = format;
        Ok(())
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
        self.generation += 1;
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

    /// Whether rows are sorted or filtered.
    pub fn has_view(&self) -> bool {
        !self.view.spec.is_empty()
    }

    /// Sorts and filters the visible rows. Fails on an invalid regular expression or column.
    pub fn set_view(&mut self, spec: ViewSpec) -> Result<()> {
        self.view = View {
            filters: self.compile_view(&spec)?,
            spec,
            rows: None,
        };
        self.refresh_view();
        Ok(())
    }

    fn compile_view(&self, spec: &ViewSpec) -> Result<Vec<CompiledFilter>> {
        let columns = spec
            .sort
            .iter()
            .map(|key| Some(key.column))
            .chain(spec.filters.iter().map(|filter| filter.column));
        for column in columns.flatten() {
            self.check_column(column)?;
        }
        spec.filters.iter().map(CompiledFilter::new).collect()
    }

    /// Prepares sorting and filtering to run off this document, on a snapshot of its rows, so a
    /// large file stays responsive. Fails at once on an invalid regular expression or column.
    pub fn begin_view(&self, spec: ViewSpec) -> Result<ViewTask> {
        let filters = self.compile_view(&spec)?;
        Ok(ViewTask {
            view: View { spec, filters, rows: None },
            snapshot: self.snapshot(),
            generation: self.generation,
            done: false,
        })
    }

    /// Shows the result of a view task. Returns `false` if the task did not finish or the rows
    /// changed since it began; the view stays as it was.
    pub fn finish_view(&mut self, task: ViewTask) -> bool {
        if !task.done || task.generation != self.generation {
            return false;
        }
        self.view = task.view;
        true
    }

    /// The rows, settings and view without the edit history, sharing the file's bytes and the
    /// row lists: work that only reads can run on it off the document, so the document stays
    /// free for the table while it runs.
    pub fn snapshot(&self) -> Document {
        Document {
            buf: Arc::clone(&self.buf),
            content_start: self.content_start,
            encoding: self.encoding,
            has_bom: self.has_bom,
            dialect: self.dialect,
            trailing_newline: self.trailing_newline,
            records: Arc::clone(&self.records),
            has_header: self.has_header,
            column_count: self.column_count,
            unterminated_quote: self.unterminated_quote,
            detection_confident: self.detection_confident,
            generation: self.generation,
            indexing: self.indexing,
            history: History::default(),
            view: self.view.clone(),
            format: self.format,
        }
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
        let order = self.view.rows.take().map(Arc::unwrap_or_clone).unwrap_or_default();
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

    pub(crate) fn refresh_view(&mut self) {
        let rows = self.view_rows(&self.view, &AtomicBool::new(false));
        self.view.rows = rows.expect("not cancelled").map(Arc::new);
    }

    /// The rows a view shows, `None` for all of them; `None` outside when cancelled.
    fn view_rows(&self, view: &View, cancel: &AtomicBool) -> Option<Option<Vec<usize>>> {
        if view.spec.is_empty() {
            return Some(None);
        }
        // Rows are checked on all cores, in chunks that stop early when cancelled.
        let mut rows: Vec<usize> = (self.header_offset()..self.records.len())
            .into_par_iter()
            .with_min_len(view::CANCEL_CHECK / 16)
            .filter(|&row| !cancel.load(Ordering::Relaxed) && self.passes(row, &view.filters))
            .collect();
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if !view.spec.sort.is_empty()
            && !view::sort_rows(&mut rows, &view.spec.sort, |row, column| self.value_at(row, column), cancel)
        {
            return None;
        }
        Some(Some(rows))
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
    pub(crate) fn reset_view(&mut self) {
        self.view = View::default();
    }

    pub(crate) fn header_offset(&self) -> usize {
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
        let splice = Splice::apply(Arc::make_mut(&mut self.records), row, 1, vec![updated]);
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
        let splice = Splice::apply(Arc::make_mut(&mut self.records), at, 0, rows);
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
        // Many scattered rows, such as duplicates, are taken out in one pass over the list.
        let runs = 1 + rows.windows(2).filter(|pair| pair[1] != pair[0] + 1).count();
        if runs > SCATTERED_RUNS {
            let splice = Splice::remove(Arc::make_mut(&mut self.records), rows);
            self.commit(vec![splice], self.column_count, true);
            self.refresh_view();
            return Ok(());
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
                Arc::make_mut(&mut self.records),
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

    /// Moves `rows` so that they stand before row `to` (an index before the move;
    /// `to == row_count` moves them to the end), keeping their order, as one undoable step.
    /// Returns `false` if nothing moves.
    pub fn move_rows(&mut self, rows: &[usize], to: usize) -> Result<bool> {
        let mut rows = rows.to_vec();
        rows.sort_unstable();
        rows.dedup();
        let Some(&last) = rows.last() else {
            return Ok(false);
        };
        self.check_row(last)?;
        if to > self.records.len() {
            return Err(Error::RowOutOfRange {
                row: to,
                rows: self.records.len(),
            });
        }
        let target = to - rows.iter().filter(|&&row| row < to).count();
        let contiguous = rows.windows(2).all(|pair| pair[1] == pair[0] + 1);
        if contiguous && target == rows[0] {
            return Ok(false);
        }
        let moved: Vec<Record> = rows.iter().map(|&row| self.records[row].clone()).collect();
        // Remove from the end so that earlier indices stay valid, then insert the block.
        let mut splices: Vec<Splice> = rows
            .iter()
            .rev()
            .map(|&row| Splice::apply(Arc::make_mut(&mut self.records), row, 1, Vec::new()))
            .collect();
        splices.push(Splice::apply(Arc::make_mut(&mut self.records), target, 0, moved));
        self.commit(splices, self.column_count, true);
        self.refresh_view();
        Ok(true)
    }

    /// Moves column `from` to index `to` of the resulting columns. Rows too short to have
    /// either column stay untouched; others get empty fields as needed. Resets the view.
    pub fn move_column(&mut self, from: usize, to: usize) -> Result<bool> {
        self.check_column(from)?;
        self.check_column(to)?;
        if from == to {
            return Ok(false);
        }
        let reach = from.max(to);
        let updated = self
            .records
            .iter()
            .map(|record| {
                let mut fields = self.fields(record);
                if record.is_blank() || fields.len() <= from.min(to) {
                    return record.clone();
                }
                if fields.len() <= reach {
                    fields.resize(reach + 1, Field::Text(String::new()));
                }
                let field = fields.remove(from);
                fields.insert(to, field);
                Record::modified(fields, record.term())
            })
            .collect();
        self.replace_records(updated, self.column_count, false);
        self.reset_view();
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Tells apart the change the next undo reverts, so that what follows a change (such as a
    /// recorded recipe step) can follow its undo and redo. `None` with nothing to undo.
    pub fn last_change_id(&self) -> Option<u64> {
        self.history.last_id()
    }

    /// Memory the undo history may hold; older steps are dropped past it, the last one stays.
    pub fn set_undo_memory_limit(&mut self, bytes: usize) {
        self.history.set_limit(bytes);
    }

    pub fn undo(&mut self) -> bool {
        match self.history.undo(Arc::make_mut(&mut self.records)) {
            Some(reverted) => {
                self.trailing_newline = reverted.trailing_newline;
                self.after_history(reverted.columns, reverted.structural);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.history.redo(Arc::make_mut(&mut self.records)) {
            Some(reverted) => {
                self.trailing_newline = reverted.trailing_newline;
                self.after_history(reverted.columns, reverted.structural);
                true
            }
            None => false,
        }
    }

    fn after_history(&mut self, columns: usize, structural: bool) {
        self.generation += 1;
        if columns != self.column_count {
            self.column_count = columns;
            self.reset_view();
        } else if structural {
            self.refresh_view();
        }
    }

    pub(crate) fn commit(&mut self, splices: Vec<Splice>, columns_after: usize, structural: bool) {
        self.commit_with_trailing(splices, columns_after, structural, self.trailing_newline);
    }

    /// Like `commit`, and also sets whether the file ends with a line break.
    pub(crate) fn commit_with_trailing(
        &mut self,
        splices: Vec<Splice>,
        columns_after: usize,
        structural: bool,
        trailing_after: bool,
    ) {
        self.generation += 1;
        self.history.record(Transaction {
            splices,
            columns_before: self.column_count,
            columns_after,
            structural,
            trailing_before: self.trailing_newline,
            trailing_after,
            bytes: 0,
            id: 0,
        });
        self.column_count = columns_after;
        self.trailing_newline = trailing_after;
    }

    pub(crate) fn replace_records(&mut self, records: Vec<Record>, columns_after: usize, structural: bool) {
        let len = self.records.len();
        let splice = Splice::apply(Arc::make_mut(&mut self.records), 0, len, records);
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
        match self.records[row].get() {
            RecordRef::Original { start, end, .. } => {
                match parser::nth_field(&self.buf[start..end], delimiter, quote, column) {
                    Some((s, e)) => self.raw_value(start + s, start + e),
                    None => Cow::Borrowed(""),
                }
            }
            RecordRef::Modified(modified) => match modified.fields.get(column) {
                Some(field) => self.field_value(field),
                None => Cow::Borrowed(""),
            },
        }
    }

    /// All values of a row. `row` must exist.
    pub(crate) fn values(&self, row: usize) -> Vec<Cow<'_, str>> {
        self.record_values(&self.records[row])
    }

    /// Values of a record, which need not be in the document yet.
    pub(crate) fn record_values<'a>(&'a self, record: &'a Record) -> Vec<Cow<'a, str>> {
        match record.get() {
            RecordRef::Original { start, end, .. } => self
                .raw_fields(start, end)
                .map(|(s, e)| self.raw_value(s, e))
                .collect(),
            RecordRef::Modified(modified) => modified
                .fields
                .iter()
                .map(|field| self.field_value(field))
                .collect(),
        }
    }

    pub(crate) fn fields(&self, record: &Record) -> Vec<Field> {
        match record.get() {
            RecordRef::Original { start, end, .. } => self
                .raw_fields(start, end)
                .map(|(start, end)| Field::Raw { start, end })
                .collect(),
            RecordRef::Modified(modified) => modified.fields.clone(),
        }
    }

    fn raw_fields(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, usize)> {
        parser::split_fields(&self.buf[start..end], self.dialect.delimiter, self.dialect.quote)
            .into_iter()
            .map(move |(s, e)| (start + s, start + e))
    }

    pub(crate) fn field_value<'a>(&'a self, field: &'a Field) -> Cow<'a, str> {
        match field {
            Field::Raw { start, end } => self.raw_value(*start, *end),
            Field::Text(text) => Cow::Borrowed(text),
        }
    }

    pub(crate) fn raw_value(&self, start: usize, end: usize) -> Cow<'_, str> {
        let encoding = self.buffer_encoding();
        match parser::unquote(&self.buf[start..end], self.dialect.quote) {
            Cow::Borrowed(bytes) => encoding.decode(bytes),
            Cow::Owned(bytes) => Cow::Owned(encoding.decode(&bytes).into_owned()),
        }
    }

    pub(crate) fn check_row(&self, row: usize) -> Result<()> {
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

/// Sorting and filtering computed off the document; see `Document::begin_view`.
pub struct ViewTask {
    view: View,
    snapshot: Document,
    generation: u64,
    done: bool,
}

impl ViewTask {
    /// Computes the rows to show. Returns `false` when `cancel` was set before it finished.
    pub fn run(&mut self, cancel: &AtomicBool) -> bool {
        match self.snapshot.view_rows(&self.view, cancel) {
            Some(rows) => {
                self.view.rows = rows.map(Arc::new);
                self.done = true;
                true
            }
            None => false,
        }
    }
}

/// Indexes the rest of a large file in steps, off the document; see `Document::open_partially`.
pub struct Indexer {
    buf: Arc<Bytes>,
    position: usize,
    delimiter: u8,
    quote: u8,
}

/// Rows indexed in one step, for `Document::append_index`.
pub struct IndexChunk {
    records: Vec<Record>,
    max_fields: usize,
    unterminated_quote: bool,
    done: bool,
}

impl IndexChunk {
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// Parts of a chunk smaller than this are not worth a thread.
const MIN_PART: usize = 4 << 20;

impl Indexer {
    /// Indexes the records that start in the next `bytes` bytes, on all cores.
    pub fn next_chunk(&mut self, bytes: usize) -> IndexChunk {
        self.next_chunk_in_parts(bytes, MIN_PART)
    }

    /// The chunk is cut at line breaks into parts that are scanned at once, each as if it began
    /// a record. Stitched in order, a part is kept when the part before it ended exactly where
    /// it begins; a value in quotes that ran over the cut makes the part scan again from where
    /// the record really starts. The result is the same as one scan from start to end.
    fn next_chunk_in_parts(&mut self, bytes: usize, min_part: usize) -> IndexChunk {
        let buf: &[u8] = &self.buf;
        let start = self.position;
        let stop = start.saturating_add(bytes).min(buf.len());
        let parts = rayon::current_num_threads().min((stop - start) / min_part.max(1)).max(1);
        let size = (stop - start) / parts;
        let mut bounds = vec![start];
        for part in 1..parts {
            let guess = start + part * size;
            if let Some(offset) = memchr::memchr(b'\n', &buf[guess..stop]) {
                let bound = guess + offset + 1;
                if bound > *bounds.last().expect("not empty") && bound < stop {
                    bounds.push(bound);
                }
            }
        }
        bounds.push(stop);
        let (delimiter, quote) = (self.delimiter, self.quote);
        let scans: Vec<parser::Scan> = bounds
            .par_windows(2)
            .map(|part| parser::scan_until(buf, part[0], delimiter, quote, part[1]))
            .collect();

        let mut chunk = IndexChunk { records: Vec::new(), max_fields: 0, unterminated_quote: false, done: false };
        let mut next = start;
        for (part, scan) in bounds.windows(2).zip(scans) {
            if next >= part[1] {
                // A record of an earlier part reached past this one.
                continue;
            }
            let scan = if part[0] == next {
                scan
            } else {
                parser::scan_until(buf, next, delimiter, quote, part[1])
            };
            next = scan.next;
            chunk.max_fields = chunk.max_fields.max(scan.max_fields);
            chunk.unterminated_quote |= scan.unterminated_quote;
            chunk.records.extend(scan.records.into_iter().map(|r| Record::original(r.start, r.end, r.term)));
        }
        self.position = next;
        chunk.done = next >= buf.len();
        chunk
    }

    /// Bytes indexed so far, and the size of the file.
    pub fn progress(&self) -> (usize, usize) {
        (self.position, self.buf.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Indexes a whole buffer in chunks of `chunk` bytes cut into parts of at least `part`.
    fn index_in_parts(text: &[u8], chunk: usize, part: usize) -> Vec<(usize, usize, Terminator)> {
        let mut indexer = Indexer {
            buf: Arc::new(Bytes::Owned(text.to_vec())),
            position: 0,
            delimiter: b',',
            quote: b'"',
        };
        let mut records = Vec::new();
        loop {
            let chunk = indexer.next_chunk_in_parts(chunk, part);
            records.extend(chunk.records.iter().map(|record| match record.get() {
                RecordRef::Original { start, end, term } => (start, end, term),
                RecordRef::Modified(_) => unreachable!(),
            }));
            if chunk.done {
                return records;
            }
        }
    }

    #[test]
    fn indexing_in_parallel_parts_matches_one_scan() {
        // Values in quotes over several lines, doubled quotes, quotes inside unquoted values,
        // CRLF and a final record without a line break, so cuts land everywhere.
        let mut text = String::new();
        for i in 0..400 {
            match i % 5 {
                0 => text.push_str(&format!("{i},\"multi\nline\nvalue {i}\",x\n")),
                1 => text.push_str(&format!("{i},5\" disk,\"say \"\"hi\"\"\"\r\n")),
                2 => text.push_str(&format!("{i},\"a,b\n\n\nc\",\n")),
                3 => text.push_str(&format!("{i},plain,row\n")),
                _ => text.push('\n'),
            }
        }
        text.push_str("last,\"open quote\nnever closed");
        let bytes = text.as_bytes();
        let one_scan: Vec<_> = parser::scan(bytes, 0, b',', b'"', None)
            .records
            .iter()
            .map(|r| (r.start, r.end, r.term))
            .collect();
        for (chunk, part) in [(usize::MAX, 1), (97, 1), (1000, 7), (4096, 50), (333, 333)] {
            assert_eq!(index_in_parts(bytes, chunk, part), one_scan, "chunk {chunk}, part {part}");
        }
    }

    #[test]
    fn a_record_takes_16_bytes() {
        assert_eq!(std::mem::size_of::<Record>(), 16);
    }

    #[test]
    fn a_record_longer_than_4_gb_keeps_its_span() {
        let record = Record::original(10, 10 + (5 << 30), Terminator::Lf);
        assert!(matches!(record, Record::Long(_)));
        assert!(matches!(record.get(), RecordRef::Original { start: 10, end, term: Terminator::Lf } if end == 10 + (5 << 30)));
        assert!(Record::original(7, 7, Terminator::None).is_blank());
    }
}

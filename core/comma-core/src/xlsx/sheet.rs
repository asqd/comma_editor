//! Reading a worksheet part: cells with their values as shown, formulas, and what the
//! app needs to lay the sheet out.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};

use super::numfmt::{DisplayLocale, NumberFormat};
use super::refs::{self, MAX_COLUMNS, MAX_ROWS};
use super::xml::{self, EventSource, attribute, flag};
use super::{ColumnLayout, SheetLayout, SheetRange};
use crate::document::{Computed, Field, FormulaField};
use crate::edit::Progress;
use crate::error::Result;
use crate::profile::date_value;

/// What a sheet's cells need from the rest of the workbook.
pub(crate) struct Context<'a> {
    pub strings: &'a [String],
    /// Number formats by style index.
    pub formats: &'a [Arc<NumberFormat>],
    pub locale: &'a DisplayLocale,
    pub date1904: bool,
}

/// Values of a sheet as the file stores them, with each cell's style: what formulas compute
/// with, where what the table shows is rounded or formatted.
#[derive(Debug, Default)]
pub(crate) struct RawSheet {
    /// Rows from the first, each up to its last cell, styled cells without values included.
    pub rows: Vec<Vec<RawCell>>,
    pub texts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RawCell {
    pub style: u32,
    pub value: RawValue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum RawValue {
    #[default]
    Empty,
    Number(f64),
    /// Index into `RawSheet::texts`.
    Text(u32),
    Bool(bool),
    /// Index into `RawSheet::texts`.
    Error(u32),
}

impl RawSheet {
    pub(crate) fn get(&self, row: usize, column: usize) -> Option<&RawCell> {
        self.rows.get(row)?.get(column)
    }
}

pub(crate) struct SheetContent {
    /// Rows from the first row of the sheet, each up to its last cell with a value.
    pub rows: Vec<Vec<Field>>,
    pub layout: SheetLayout,
    /// Formulas, merged cells, conditional formats, validations, links, tables, drawings or
    /// a filter refer to rows of the sheet, so they cannot move on their own.
    pub refers_to_rows: bool,
    /// When asked for instead of the rows: the values as stored.
    pub raw: Option<RawSheet>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellType {
    Number,
    SharedString,
    InlineString,
    /// The text result of a formula.
    FormulaString,
    Boolean,
    Error,
    /// An ISO 8601 date and time.
    Date,
}

struct Cell {
    row: u32,
    column: u32,
    kind: CellType,
    style: usize,
    value: String,
    formula: Option<String>,
    array: Option<String>,
}

/// Cells read between looks at `Progress`.
const PROGRESS_STEP: usize = 4096;

/// What the reading loop does after an event, once the event is gone.
enum Next {
    Nothing,
    Value,
    InlineString,
    Formula(FormulaAttributes),
    CellEnd,
    End,
}

struct FormulaAttributes {
    kind: Option<String>,
    shared_index: Option<String>,
    range: Option<String>,
    empty: bool,
}

/// The sheet read so far.
struct Reading {
    rows: Vec<Vec<Field>>,
    layout: SheetLayout,
    /// Formulas shared by ranges of cells: the text and the cell it is written for, by `si`.
    shared: HashMap<String, (String, u32, u32)>,
    row: u32,
    next_row: u32,
    next_column: u32,
    cell: Option<Cell>,
    refers_to_rows: bool,
    raw: Option<RawSheet>,
    /// Reading a run of rows on its own: rows are kept from `base`, the first one's number,
    /// and shared formulas written in earlier runs are left to fill in.
    run: bool,
    base: Option<u32>,
    /// Cells of shared formulas whose formula an earlier run has: row, column and `si`.
    pending: Vec<(u32, u32, String)>,
    /// The run cannot be read on its own: a row without its number, or rows out of order.
    unsuited: bool,
}

/// Reads the sheet from `source`, which holds `size` bytes; `None` if `progress` was cancelled.
/// With `raw`, reads the values as stored instead of as shown.
pub(crate) fn read_sheet(
    source: &mut impl EventSource,
    size: u64,
    part: &str,
    context: &Context,
    progress: Option<&Progress>,
    raw: bool,
) -> Result<Option<SheetContent>> {
    let mut reading = Reading::new(raw);
    if let Some(progress) = progress {
        progress.expect(size as usize);
    }
    if !reading.read(source, part, context, progress, size)? {
        return Ok(None);
    }
    Ok(Some(reading.content()))
}

impl Reading {
    fn new(raw: bool) -> Self {
        Self {
            rows: Vec::new(),
            layout: SheetLayout::default(),
            shared: HashMap::new(),
            row: 0,
            next_row: 0,
            next_column: 0,
            cell: None,
            refers_to_rows: false,
            raw: raw.then(RawSheet::default),
            run: false,
            base: None,
            pending: Vec::new(),
            unsuited: false,
        }
    }

    fn content(self) -> SheetContent {
        let refers_to_rows = self.refers_to_rows || !self.layout.merged.is_empty();
        SheetContent { rows: self.rows, layout: self.layout, refers_to_rows, raw: self.raw }
    }

    /// Reads the events of `source`, which holds `size` bytes, telling `progress` how far it
    /// got; `false` if `progress` was cancelled.
    fn read(&mut self, source: &mut impl EventSource, part: &str, context: &Context, progress: Option<&Progress>, size: u64) -> Result<bool> {
        let mut cells = 0usize;
        let mut reported = 0u64;
        loop {
            // An event read from a stream lives in the stream's buffer: what it says is taken
            // out before anything more is read.
            let next = {
                let event = source.next_event(part)?;
                self.take(&event)
            };
            match next {
                Next::Nothing => {}
                Next::Value => match &mut self.cell {
                    Some(cell) => push_text_until(source, part, "v", &mut cell.value)?,
                    None => push_text_until(source, part, "v", &mut String::new())?,
                },
                Next::InlineString => {
                    let value = xml::read_string(source, part, "is")?;
                    if let Some(cell) = &mut self.cell {
                        cell.value = value;
                    }
                }
                Next::Formula(attributes) => {
                    let mut text = String::new();
                    if !attributes.empty {
                        push_text_until(source, part, "f", &mut text)?;
                        text = xml::unescape_characters(text);
                    }
                    if let Some(cell) = &mut self.cell {
                        let shared_index = attributes.shared_index.clone();
                        read_formula(cell, attributes, text, &mut self.shared);
                        // A shared formula written in an earlier run is filled in afterwards.
                        if self.run && cell.formula.is_none() && let Some(index) = shared_index {
                            self.pending.push((cell.row, cell.column, index));
                            cell.formula = Some(String::new());
                        }
                    }
                }
                Next::CellEnd => {
                    if let Some(cell) = &mut self.cell {
                        let base = self.base.unwrap_or(0);
                        if cell.row < base {
                            self.unsuited = true;
                        } else {
                            match &mut self.raw {
                                Some(raw) => place_raw(raw, base, cell, context),
                                None => place(&mut self.rows, base, cell, context),
                            }
                        }
                    }
                    cells += 1;
                    if cells.is_multiple_of(PROGRESS_STEP)
                        && let Some(progress) = progress
                    {
                        if progress.is_cancelled() {
                            return Ok(false);
                        }
                        let position = source.position().min(size);
                        progress.advance((position - reported) as usize);
                        reported = position;
                    }
                }
                Next::End => break,
            }
        }
        if let Some(progress) = progress {
            progress.advance(size.saturating_sub(reported) as usize);
        }
        Ok(true)
    }

    /// Takes in an event, and says what to read next.
    fn take(&mut self, event: &Event<'_>) -> Next {
        let (element, empty) = match event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(element) if element.local_name().as_ref() == "c" => return Next::CellEnd,
            Event::Eof => return Next::End,
            _ => return Next::Nothing,
        };
        match element.local_name().as_ref() {
            "row" => {
                let number = attribute(element, "r").and_then(|r| r.parse::<u32>().ok()).filter(|&r| (1..=MAX_ROWS).contains(&r));
                if self.run {
                    match number {
                        Some(number) if self.base.is_none() => self.base = Some(number - 1),
                        Some(_) => {}
                        None => self.unsuited = true,
                    }
                }
                self.row = number.map_or(self.next_row, |r| r - 1);
                self.next_row = self.row + 1;
                self.next_column = 0;
            }
            "c" => {
                let (position, kind, style) = cell_attributes(element);
                let (row, column) = position.unwrap_or((self.row, self.next_column));
                self.next_column = column + 1;
                if !empty && row < MAX_ROWS && column < MAX_COLUMNS {
                    // The value's buffer passes from cell to cell.
                    let mut value = self.cell.take().map(|cell| cell.value).unwrap_or_default();
                    value.clear();
                    self.cell = Some(Cell { row, column, kind, style, value, formula: None, array: None });
                } else {
                    self.cell = None;
                }
            }
            "v" if !empty => return Next::Value,
            "is" if !empty => return Next::InlineString,
            "conditionalFormatting" | "dataValidation" | "hyperlink" | "autoFilter" => self.refers_to_rows = true,
            // Tables, drawings and comments follow rows and columns when saved.
            "tablePart" | "drawing" | "legacyDrawing" => self.refers_to_rows = true,
            "f" => {
                self.refers_to_rows = true;
                let owned = |name: &str| attribute(element, name).map(|value| value.into_owned());
                return Next::Formula(FormulaAttributes {
                    kind: owned("t"),
                    shared_index: owned("si"),
                    range: owned("ref"),
                    empty,
                });
            }
            "mergeCell" => {
                if let Some(((top, left), (bottom, right))) = attribute(element, "ref").and_then(|r| refs::parse_range(&r)) {
                    self.layout.merged.push(SheetRange {
                        top: top as usize,
                        left: left as usize,
                        bottom: bottom as usize,
                        right: right as usize,
                    });
                }
            }
            "col" => {
                let number = |name: &str| attribute(element, name).and_then(|value| value.parse::<usize>().ok());
                if let (Some(first), Some(last)) = (number("min"), number("max"))
                    && first >= 1
                    && first <= last
                {
                    self.layout.columns.push(ColumnLayout {
                        first: first - 1,
                        last: (last - 1).min(MAX_COLUMNS as usize - 1),
                        width: attribute(element, "width").and_then(|width| width.parse().ok()),
                        hidden: flag(element, "hidden"),
                    });
                }
            }
            "sheetFormatPr" => {
                self.layout.default_column_width = attribute(element, "defaultColWidth").and_then(|width| width.parse().ok());
            }
            "pane" => {
                if matches!(attribute(element, "state").as_deref(), Some("frozen" | "frozenSplit")) {
                    let split = |name: &str| attribute(element, name).and_then(|value| value.parse::<f64>().ok()).unwrap_or(0.0);
                    self.layout.frozen_columns = split("xSplit") as usize;
                    self.layout.frozen_rows = split("ySplit") as usize;
                }
            }
            _ => {}
        }
        Next::Nothing
    }
}

/// Bytes of a sheet from which its rows are read on several threads.
pub(crate) const PARALLEL_SIZE: u64 = 8 << 20;

/// Bytes of rows a thread reads at a time.
const RUN: usize = 1 << 20;

/// What reading a sheet on several threads came to.
pub(crate) enum Parallel {
    /// The sheet, or `None` if `progress` was cancelled.
    Read(Option<SheetContent>),
    /// The sheet is not laid out for it, its rows unnumbered or out of order, and is to be read
    /// on one thread; nothing was read.
    Unsuited,
}

/// A run of whole rows of a sheet, cut from it as it streams in.
struct Run {
    index: usize,
    /// Where the run starts in the part.
    offset: usize,
    bytes: Vec<u8>,
}

/// Reads a sheet streaming from `input`, `size` bytes, as `read_sheet` does, its rows in runs
/// read on several threads while the part is still coming. What comes before and after the
/// rows, the columns and merged cells among them, is read on this thread.
pub(crate) fn read_sheet_parallel(input: &mut impl Read, size: u64, part: &str, context: &Context, progress: Option<&Progress>, raw: bool) -> Result<Parallel> {
    read_in_runs(input, size, part, context, progress, raw, RUN)
}

fn read_in_runs(input: &mut impl Read, size: u64, part: &str, context: &Context, progress: Option<&Progress>, raw: bool, run_size: usize) -> Result<Parallel> {
    let spare = std::sync::Mutex::new(Vec::new());
    let mut feed = Feed { input, buffer: Vec::with_capacity(run_size * 2), eof: false, read: 0, spare: &spare };
    if let Some(progress) = progress {
        progress.expect(size as usize);
    }
    let unsuited = |read: usize| {
        if let Some(progress) = progress {
            progress.take_back(size as usize, read);
        }
        Ok(Parallel::Unsuited)
    };

    // The start, up to the rows: the root and `<sheetData>`, with their prefixes.
    let (data_end, root, data_name) = loop {
        if let Some(found) = find_start_tag(&feed.buffer, b"sheetData") {
            break found;
        }
        if feed.eof || feed.buffer.len() > 64 << 20 || !feed.more(progress)? {
            return unsuited(feed.read);
        }
    };
    if feed.buffer[..data_end].ends_with(b"/>") || feed.buffer.starts_with(&[0xFF, 0xFE]) || feed.buffer.starts_with(&[0xFE, 0xFF]) {
        return unsuited(feed.read);
    }
    let Some(root) = root else { return unsuited(feed.read) };
    let prefix = data_name.strip_suffix(b"sheetData").unwrap_or_default().to_vec();
    let row_end = [b"</".as_slice(), &prefix, b"row>"].concat();
    let data_close = [b"</".as_slice(), &data_name, b">"].concat();
    let mut reading = Reading::new(raw);
    let Ok(head) = std::str::from_utf8(&feed.buffer[..data_end]) else { return unsuited(feed.read) };
    let head = head.strip_prefix('\u{feff}').unwrap_or(head);
    reading.read(&mut xml::reader(head), part, context, None, 0)?;
    feed.buffer.drain(..data_end);
    let mut consumed = data_end;

    // Runs of rows to the threads as they come, the rest after `</sheetData>` kept.
    let threads = std::thread::available_parallelism().map_or(1, |count| count.get()).clamp(1, 16);
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Run>(threads);
    let receiver = std::sync::Mutex::new(receiver);
    let results = std::sync::Mutex::new(Vec::new());
    let produced: Result<Option<Vec<u8>>> = std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let run = match receiver.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).recv() {
                        Ok(run) => run,
                        Err(_) => break,
                    };
                    let rows = read_run(&run, part, context, raw);
                    results.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push((run.index, rows));
                    spare.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(run.bytes);
                }
            });
        }
        let mut index = 0;
        let mut searched = 0;
        let produced = loop {
            if let Some(at) = memchr::memmem::find(&feed.buffer[searched..], &data_close).map(|at| searched + at) {
                let bytes = feed.take(at);
                if !bytes.is_empty() && sender.send(Run { index, offset: consumed, bytes }).is_err() {
                    break Ok(None);
                }
                // The rest of the part, read here.
                while !feed.eof {
                    if !feed.more(progress)? {
                        break;
                    }
                }
                break Ok(Some(std::mem::take(&mut feed.buffer)));
            }
            if feed.buffer.len() >= run_size
                && let Some(at) = memchr::memmem::rfind(&feed.buffer, &row_end)
            {
                let bytes = feed.take(at + row_end.len());
                let length = bytes.len();
                if sender.send(Run { index, offset: consumed, bytes }).is_err() {
                    break Ok(None);
                }
                consumed += length;
                index += 1;
                searched = 0;
                continue;
            }
            if feed.eof {
                break Ok(None);
            }
            searched = feed.buffer.len().saturating_sub(data_close.len());
            match feed.more(progress) {
                Ok(true) => {}
                Ok(false) => break Ok(None),
                Err(error) => break Err(error),
            }
        };
        drop(sender);
        produced
    });
    let tail = match produced? {
        Some(tail) => tail,
        None if progress.is_some_and(Progress::is_cancelled) => return Ok(Parallel::Read(None)),
        None => return unsuited(feed.read),
    };

    // The runs in their order, shared formulas filled in from the runs before.
    let mut runs = results.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    runs.sort_by_key(|(index, _)| *index);
    let mut shared: HashMap<String, (String, u32, u32)> = HashMap::new();
    for (_, run) in runs {
        let run = run?;
        if run.unsuited {
            return unsuited(feed.read);
        }
        let base = run.base.unwrap_or(0) as usize;
        match (&mut reading.raw, run.raw) {
            (Some(raw), Some(mut more)) if !more.rows.is_empty() => {
                if base < raw.rows.len() {
                    return unsuited(feed.read);
                }
                let offset = raw.texts.len() as u32;
                for cell in more.rows.iter_mut().flatten() {
                    match &mut cell.value {
                        RawValue::Text(index) | RawValue::Error(index) => *index += offset,
                        _ => {}
                    }
                }
                raw.rows.resize_with(base, Vec::new);
                raw.rows.append(&mut more.rows);
                raw.texts.append(&mut more.texts);
            }
            (Some(_), _) => {}
            (None, _) => {
                if !run.rows.is_empty() {
                    if base < reading.rows.len() {
                        return unsuited(feed.read);
                    }
                    reading.rows.resize_with(base, Vec::new);
                    reading.rows.extend(run.rows);
                }
                for (row, column, index) in run.pending {
                    let Some(Field::Formula(cell)) = reading.rows.get_mut(row as usize).and_then(|fields| fields.get_mut(column as usize)) else { continue };
                    match shared.get(&index) {
                        Some((formula, from_row, from_column)) => {
                            cell.formula = refs::shift_formula(formula, i64::from(row) - i64::from(*from_row), i64::from(column) - i64::from(*from_column));
                        }
                        None => {
                            let value = std::mem::take(&mut cell.value);
                            reading.rows[row as usize][column as usize] = Field::Text(value);
                        }
                    }
                }
            }
        }
        reading.refers_to_rows |= run.refers_to_rows;
        shared.extend(run.shared);
    }

    // What comes after the rows, read inside the elements it closes.
    let mut text = Vec::with_capacity(tail.len() + root.len() + data_name.len() + 4);
    text.extend_from_slice(b"<");
    text.extend_from_slice(&root);
    text.extend_from_slice(b"><");
    text.extend_from_slice(&data_name);
    text.extend_from_slice(b">");
    text.extend_from_slice(&tail);
    let Ok(text) = std::str::from_utf8(&text) else { return unsuited(feed.read) };
    reading.read(&mut xml::reader(text), part, context, None, 0)?;
    if let Some(progress) = progress {
        progress.advance(size.saturating_sub(feed.read as u64) as usize);
    }
    Ok(Parallel::Read(Some(reading.content())))
}

/// The part as it streams in.
struct Feed<'a, R> {
    input: &'a mut R,
    buffer: Vec<u8>,
    eof: bool,
    read: usize,
    /// Buffers of runs read, to take the next runs: a buffer freed for each run would stay
    /// with the allocator, as large as the sheet.
    spare: &'a std::sync::Mutex<Vec<Vec<u8>>>,
}

impl<R: Read> Feed<'_, R> {
    /// The first `length` bytes, taken out into a buffer of a run read before if there is one.
    fn take(&mut self, length: usize) -> Vec<u8> {
        let mut bytes = self.spare.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).pop().unwrap_or_default();
        bytes.clear();
        bytes.extend_from_slice(&self.buffer[..length]);
        self.buffer.drain(..length);
        bytes
    }

    /// Reads more into the buffer; `false` if `progress` was cancelled.
    fn more(&mut self, progress: Option<&Progress>) -> Result<bool> {
        if progress.is_some_and(Progress::is_cancelled) {
            return Ok(false);
        }
        let start = self.buffer.len();
        self.buffer.resize(start + (1 << 20), 0);
        let count = loop {
            match self.input.read(&mut self.buffer[start..]) {
                Ok(count) => break count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    self.buffer.truncate(start);
                    return Err(error.into());
                }
            }
        };
        self.buffer.truncate(start + count);
        self.eof = count == 0;
        self.read += count;
        if let Some(progress) = progress {
            progress.advance(count);
        }
        Ok(true)
    }
}

/// Rows of one run, read on their own.
struct RunRows {
    rows: Vec<Vec<Field>>,
    raw: Option<RawSheet>,
    base: Option<u32>,
    refers_to_rows: bool,
    shared: HashMap<String, (String, u32, u32)>,
    pending: Vec<(u32, u32, String)>,
    unsuited: bool,
}

fn read_run(run: &Run, part: &str, context: &Context, raw: bool) -> Result<RunRows> {
    let mut reading = Reading::new(raw);
    reading.run = true;
    match std::str::from_utf8(&run.bytes) {
        Ok(text) => {
            if let Err(error) = reading.read(&mut xml::reader(text), part, context, None, 0) {
                // Where in the part, not in the run.
                return Err(match error {
                    crate::error::Error::Workbook(crate::error::WorkbookError::Damaged { part, position, detail }) => {
                        let position = position.map(|position| position + run.offset as u64);
                        crate::error::WorkbookError::Damaged { part, position, detail }.into()
                    }
                    error => error,
                });
            }
        }
        Err(_) => reading.unsuited = true,
    }
    Ok(RunRows {
        rows: reading.rows,
        raw: reading.raw,
        base: reading.base,
        refers_to_rows: reading.refers_to_rows,
        shared: reading.shared,
        pending: reading.pending,
        unsuited: reading.unsuited,
    })
}

/// The end of the first start tag with this local name, the name of the first element of all
/// before it, and the tag's name with its prefix.
fn find_start_tag(xml: &[u8], local: &[u8]) -> Option<(usize, Option<Vec<u8>>, Vec<u8>)> {
    let name_byte = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
    let mut from = 0;
    while let Some(at) = memchr::memmem::find(&xml[from..], local).map(|at| from + at) {
        from = at + 1;
        let after = at + local.len();
        if !matches!(xml.get(after), Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n')) {
            continue;
        }
        let mut start = at;
        if start > 0 && xml[start - 1] == b':' {
            start -= 1;
            while start > 0 && name_byte(xml[start - 1]) {
                start -= 1;
            }
        }
        if start == 0 || xml[start - 1] != b'<' {
            continue;
        }
        let end = after + memchr::memchr(b'>', &xml[after..])? + 1;
        let root = first_element(&xml[..start - 1]);
        return Some((end, root, xml[start..after].to_vec()));
    }
    None
}

/// The name of the first element in `xml`, past the declaration and comments.
fn first_element(xml: &[u8]) -> Option<Vec<u8>> {
    let mut at = 0;
    loop {
        at += memchr::memchr(b'<', &xml[at..])?;
        match xml.get(at + 1) {
            Some(b'?' | b'!') => at += 1,
            Some(_) => {
                let name: Vec<u8> = xml[at + 1..].iter().copied().take_while(|byte| !byte.is_ascii_whitespace() && *byte != b'>' && *byte != b'/').collect();
                return (!name.is_empty()).then_some(name);
            }
            None => return None,
        }
    }
}

/// A formula of a cell: its own, an array formula, or one shared with other cells and moved to
/// this one.
fn read_formula(cell: &mut Cell, attributes: FormulaAttributes, text: String, shared: &mut HashMap<String, (String, u32, u32)>) {
    match attributes.kind.as_deref() {
        Some("shared") => {
            let Some(index) = attributes.shared_index else {
                return;
            };
            if !text.is_empty() {
                shared.insert(index, (text.clone(), cell.row, cell.column));
                cell.formula = Some(text);
            } else if let Some((formula, row, column)) = shared.get(&index) {
                cell.formula = Some(refs::shift_formula(
                    formula,
                    i64::from(cell.row) - i64::from(*row),
                    i64::from(cell.column) - i64::from(*column),
                ));
            }
        }
        Some("array") => {
            cell.array = attributes.range;
            cell.formula = Some(text);
        }
        // What-if tables keep their results only.
        Some("dataTable") => {}
        _ if !text.is_empty() => cell.formula = Some(text),
        _ => {}
    }
}

/// Where a cell is, its type and its style, read in one pass over its attributes. None of
/// them can hold an entity, so their raw text is their value.
pub(crate) fn cell_attributes(element: &BytesStart<'_>) -> (Option<(u32, u32)>, CellType, usize) {
    let (mut position, mut kind, mut style) = (None, CellType::Number, 0);
    for attribute in element.attributes().with_checks(false).flatten() {
        match attribute.key.local_name().as_ref() {
            "r" => position = refs::parse_cell(&attribute.value),
            "t" => {
                kind = match &*attribute.value {
                    "s" => CellType::SharedString,
                    "inlineStr" => CellType::InlineString,
                    "str" => CellType::FormulaString,
                    "b" => CellType::Boolean,
                    "e" => CellType::Error,
                    "d" => CellType::Date,
                    _ => CellType::Number,
                }
            }
            "s" => style = attribute.value.parse().unwrap_or(0),
            _ => {}
        }
    }
    (position, kind, style)
}

/// Adds the plain text up to the end of element `end` to `out`.
fn push_text_until(source: &mut impl EventSource, part: &str, end: &str, out: &mut String) -> Result<()> {
    loop {
        let event = source.next_event(part)?;
        match &event {
            Event::End(element) if element.local_name().as_ref() == end => return Ok(()),
            Event::Eof => return Ok(()),
            event => xml::push_text(out, event),
        }
    }
}

/// Puts a cell into its row, as the value Excel shows. Cells with neither a value nor a
/// formula, which only carry a style, are left out.
fn place(rows: &mut Vec<Vec<Field>>, base: u32, cell: &mut Cell, context: &Context) {
    if cell.value.is_empty() && cell.formula.is_none() {
        return;
    }
    let value = shown_value(cell, context);
    let field = match cell.formula.take() {
        Some(formula) => {
            let computed = match raw_value(cell, context, &mut Vec::new()) {
                RawValue::Number(number) => Computed::Number(number),
                RawValue::Bool(truth) => Computed::Bool(truth),
                RawValue::Text(_) => Computed::Text(cell.value.clone()),
                RawValue::Error(_) => Computed::Error(cell.value.clone()),
                RawValue::Empty => Computed::Unknown,
            };
            Field::Formula(Box::new(FormulaField { formula, value, array: cell.array.take(), computed, from_file: true, date_format: None }))
        }
        None => Field::Text(value),
    };
    let (row, column) = ((cell.row - base) as usize, cell.column as usize);
    if rows.len() <= row {
        // Rows of a sheet are mostly as long as the one before.
        let width = rows.last().map_or(0, Vec::len);
        rows.resize_with(row + 1, Vec::new);
        rows[row].reserve(width);
    }
    let fields = &mut rows[row];
    if fields.len() <= column {
        fields.resize(column + 1, Field::Text(String::new()));
    }
    fields[column] = field;
}

/// Puts a cell's stored value and style into its row. Cells with a style only are kept: a
/// formula typed into one takes its format.
fn place_raw(raw: &mut RawSheet, base: u32, cell: &mut Cell, context: &Context) {
    let value = raw_value(cell, context, &mut raw.texts);
    let (row, column) = ((cell.row - base) as usize, cell.column as usize);
    if raw.rows.len() <= row {
        raw.rows.resize_with(row + 1, Vec::new);
    }
    let cells = &mut raw.rows[row];
    if cells.len() <= column {
        cells.resize(column + 1, RawCell::default());
    }
    cells[column] = RawCell { style: cell.style as u32, value };
}

/// A cell's value as stored, its text added to `texts`.
fn raw_value(cell: &Cell, context: &Context, texts: &mut Vec<String>) -> RawValue {
    let mut text = |value: String| {
        texts.push(value);
        texts.len() as u32 - 1
    };
    let raw = cell.value.trim();
    match cell.kind {
        _ if cell.value.is_empty() => RawValue::Empty,
        CellType::SharedString => {
            let shared = raw.parse::<usize>().ok().and_then(|index| context.strings.get(index)).cloned().unwrap_or_default();
            RawValue::Text(text(shared))
        }
        CellType::InlineString | CellType::FormulaString => RawValue::Text(text(cell.value.clone())),
        CellType::Boolean => RawValue::Bool(raw == "1" || raw == "true"),
        CellType::Error => RawValue::Error(text(cell.value.clone())),
        CellType::Date => match date_value(raw) {
            Some(days) => RawValue::Number(days + if context.date1904 { 24_107.0 } else { 25_569.0 }),
            None => RawValue::Text(text(cell.value.clone())),
        },
        CellType::Number => match raw.parse::<f64>() {
            Ok(number) => RawValue::Number(number),
            Err(_) => RawValue::Text(text(cell.value.clone())),
        },
    }
}

fn shown_value(cell: &Cell, context: &Context) -> String {
    let format = context.formats.get(cell.style).unwrap_or(&context.formats[0]);
    let raw = &cell.value;
    match cell.kind {
        CellType::SharedString => {
            let text = raw.trim().parse::<usize>().ok().and_then(|index| context.strings.get(index));
            format.format_text(text.map_or("", String::as_str))
        }
        CellType::InlineString | CellType::FormulaString => format.format_text(raw),
        CellType::Boolean => match raw.trim() {
            "1" | "true" => context.locale.true_text.clone(),
            "" => String::new(),
            _ => context.locale.false_text.clone(),
        },
        CellType::Error => raw.clone(),
        CellType::Date => match date_value(raw) {
            Some(days) => {
                let serial = days + if context.date1904 { 24_107.0 } else { 25_569.0 };
                format.format_number(serial, context.locale, context.date1904)
            }
            None => raw.clone(),
        },
        CellType::Number => match raw.trim().parse::<f64>() {
            Ok(number) => format.format_number(number, context.locale, context.date1904),
            Err(_) => raw.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet with what reading in runs has to put together: columns and a frozen pane
    /// before the rows, a shared formula written early and used late, inline strings, gaps
    /// between rows, merged cells and a link after them.
    fn sheet(prefix: &str) -> String {
        let p = prefix;
        let mut rows = String::new();
        for row in 1..=60u32 {
            if row % 7 == 0 {
                continue;
            }
            let formula = if row == 2 {
                format!(r#"<{p}f t="shared" ref="C2:C60" si="0">A2*2</{p}f>"#)
            } else {
                format!(r#"<{p}f t="shared" si="0"/>"#)
            };
            rows.push_str(&format!(
                r#"<{p}row r="{row}" spans="1:3"><{p}c r="A{row}"><{p}v>{row}</{p}v></{p}c><{p}c r="B{row}" t="inlineStr"><{p}is><{p}t>Строка &amp; {row}</{p}t></{p}is></{p}c>{}</{p}row>"#,
                if row >= 2 { format!(r#"<{p}c r="C{row}">{formula}<{p}v>{}</{p}v></{p}c>"#, row * 2) } else { String::new() }
            ));
        }
        let space = if p.is_empty() { String::new() } else { format!(r#" xmlns:{}="m""#, &p[..p.len() - 1]) };
        format!(
            "\u{feff}<?xml version=\"1.0\"?>\n<{p}worksheet{space} xmlns:r=\"r\"><{p}sheetViews><{p}sheetView><{p}pane ySplit=\"1\" state=\"frozen\"/></{p}sheetView></{p}sheetViews><{p}cols><{p}col min=\"2\" max=\"2\" width=\"30\"/></{p}cols><{p}sheetData>{rows}</{p}sheetData><{p}mergeCells><{p}mergeCell ref=\"D1:E2\"/></{p}mergeCells><{p}hyperlinks><{p}hyperlink ref=\"A1\" r:id=\"x\"/></{p}hyperlinks></{p}worksheet>"
        )
    }

    fn read_both(xml: &str, raw: bool) -> (String, String) {
        let formats = [Arc::new(NumberFormat::general())];
        let locale = DisplayLocale::default();
        let context = Context { strings: &[], formats: &formats, locale: &locale, date1904: false };
        let text = xml.strip_prefix('\u{feff}').unwrap_or(xml);
        let one = read_sheet(&mut xml::reader(text), xml.len() as u64, "sheet", &context, None, raw).unwrap().unwrap();
        let Parallel::Read(Some(runs)) = read_in_runs(&mut xml.as_bytes(), xml.len() as u64, "sheet", &context, None, raw, 300).unwrap() else {
            panic!("not read in runs");
        };
        let show = |content: &SheetContent| format!("{:?}\n{:?}\n{:?}\n{}", content.rows, content.raw, content.layout, content.refers_to_rows);
        (show(&one), show(&runs))
    }

    #[test]
    fn rows_read_in_runs_are_the_rows_read_at_once() {
        for prefix in ["", "x:"] {
            for raw in [false, true] {
                let (one, runs) = read_both(&sheet(prefix), raw);
                assert_eq!(one, runs, "prefix {prefix:?}, raw {raw}");
            }
        }
        // The shared formula reaches the last runs, the merged cells and pane come through.
        let (_, runs) = read_both(&sheet(""), false);
        for part in ["A59*2", "Строка & 59", "SheetRange { top: 0, left: 3, bottom: 1, right: 4 }", "frozen_rows: 1"] {
            assert!(runs.contains(part), "{part} in {runs}");
        }
    }

    #[test]
    fn unnumbered_rows_are_read_at_once() {
        let xml = sheet("").replace(r#"<row r="30" spans="1:3">"#, "<row>");
        let formats = [Arc::new(NumberFormat::general())];
        let locale = DisplayLocale::default();
        let context = Context { strings: &[], formats: &formats, locale: &locale, date1904: false };
        let read = read_in_runs(&mut xml.as_bytes(), xml.len() as u64, "sheet", &context, None, false, 300).unwrap();
        assert!(matches!(read, Parallel::Unsuited));
    }
}

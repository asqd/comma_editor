//! Reading a worksheet part: cells with their values as shown, formulas, and what the
//! app needs to lay the sheet out.

use std::collections::HashMap;
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
    /// Tables, drawings or comments are anchored to cells: rows and columns stay where they are.
    pub locked: bool,
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
    locked: bool,
    raw: Option<RawSheet>,
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
    let mut reading = Reading {
        rows: Vec::new(),
        layout: SheetLayout::default(),
        shared: HashMap::new(),
        row: 0,
        next_row: 0,
        next_column: 0,
        cell: None,
        refers_to_rows: false,
        locked: false,
        raw: raw.then(RawSheet::default),
    };
    let mut cells = 0usize;
    let mut reported = 0u64;
    if let Some(progress) = progress {
        progress.expect(size as usize);
    }
    loop {
        // An event read from a stream lives in the stream's buffer: what it says is taken out
        // before anything more is read.
        let next = {
            let event = source.next_event(part)?;
            reading.take(&event)
        };
        match next {
            Next::Nothing => {}
            Next::Value => match &mut reading.cell {
                Some(cell) => push_text_until(source, part, "v", &mut cell.value)?,
                None => push_text_until(source, part, "v", &mut String::new())?,
            },
            Next::InlineString => {
                let value = xml::read_string(source, part, "is")?;
                if let Some(cell) = &mut reading.cell {
                    cell.value = value;
                }
            }
            Next::Formula(attributes) => {
                let mut text = String::new();
                if !attributes.empty {
                    push_text_until(source, part, "f", &mut text)?;
                    text = xml::unescape_characters(text);
                }
                if let Some(cell) = &mut reading.cell {
                    read_formula(cell, attributes, text, &mut reading.shared);
                }
            }
            Next::CellEnd => {
                if let Some(cell) = &mut reading.cell {
                    match &mut reading.raw {
                        Some(raw) => place_raw(raw, cell, context),
                        None => place(&mut reading.rows, cell, context),
                    }
                }
                cells += 1;
                if cells.is_multiple_of(PROGRESS_STEP)
                    && let Some(progress) = progress
                {
                    if progress.is_cancelled() {
                        return Ok(None);
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
    let refers_to_rows = reading.refers_to_rows || !reading.layout.merged.is_empty();
    Ok(Some(SheetContent { rows: reading.rows, layout: reading.layout, refers_to_rows, locked: reading.locked, raw: reading.raw }))
}

impl Reading {
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
                self.row = attribute(element, "r")
                    .and_then(|r| r.parse::<u32>().ok())
                    .filter(|&r| (1..=MAX_ROWS).contains(&r))
                    .map_or(self.next_row, |r| r - 1);
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
            "tablePart" | "drawing" | "legacyDrawing" => {
                self.refers_to_rows = true;
                self.locked = true;
            }
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
fn place(rows: &mut Vec<Vec<Field>>, cell: &mut Cell, context: &Context) {
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
            Field::Formula(Box::new(FormulaField { formula, value, array: cell.array.take(), computed, from_file: true }))
        }
        None => Field::Text(value),
    };
    let (row, column) = (cell.row as usize, cell.column as usize);
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
fn place_raw(raw: &mut RawSheet, cell: &mut Cell, context: &Context) {
    let value = raw_value(cell, context, &mut raw.texts);
    let (row, column) = (cell.row as usize, cell.column as usize);
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

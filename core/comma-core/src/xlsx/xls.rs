//! Excel 97–2003 workbooks (`.xls`): an OLE compound file whose `Workbook` stream holds BIFF8
//! records. Values and strings come from `calamine`; what it keeps to itself is read here from
//! the records: number formats of cells, column widths, frozen panes, merged cells, the active
//! sheet.
//!
//! Cells show the results Excel stored, without their formulas: `calamine` writes ranges and
//! columns past Z wrongly when it turns BIFF formulas back into text (`SUM(D2:D4)` comes out as
//! `SUM($USP$2:$USP$4)`), and a wrong formula is worse than none.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use calamine::{Data, Reader, SheetType, SheetVisible, Xls, XlsError};

use super::package::Shared;
use super::sheet::Context;
use super::styles::Styles;
use super::{ColumnLayout, SheetEntry, SheetKind, SheetLayout, SheetRange, SheetVisibility};
use crate::bytes::Bytes;
use crate::document::Field;
use crate::error::{Result, WorkbookError};

/// Signature of an OLE compound file: an `.xls` workbook, or an encrypted `.xlsx`.
pub(crate) const COMPOUND_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub(crate) struct XlsBook {
    calamine: Mutex<Xls<Cursor<Shared>>>,
    /// The `Workbook` stream, for the records `calamine` does not expose.
    stream: Vec<u8>,
    /// Where each sheet's records begin, by sheet.
    offsets: Vec<usize>,
    pub(crate) styles: Styles,
}

/// What opening a compound file finds.
pub(crate) struct Opened {
    pub book: XlsBook,
    pub sheets: Vec<SheetEntry>,
    pub active: usize,
    pub date1904: bool,
}

// Record types.
const EOF: u16 = 0x000A;
const DATEMODE: u16 = 0x0022;
const WINDOW1: u16 = 0x003D;
const PANE: u16 = 0x0041;
const DEFCOLWIDTH: u16 = 0x0055;
const COLINFO: u16 = 0x007D;
const BOUNDSHEET: u16 = 0x0085;
const MULRK: u16 = 0x00BD;
const XF: u16 = 0x00E0;
const MERGEDCELLS: u16 = 0x00E5;
const NUMBER: u16 = 0x0203;
const WINDOW2: u16 = 0x023E;
const RK: u16 = 0x027E;
const FORMAT: u16 = 0x041E;
const FORMULA: u16 = 0x0006;

pub(crate) fn open(bytes: Bytes) -> Result<Opened> {
    let shared = Shared(Arc::new(bytes));
    let mut compound = cfb::CompoundFile::open(Cursor::new(shared.clone())).map_err(|_| WorkbookError::NotAWorkbook)?;
    // An encrypted .xlsx is a compound file too, with the encrypted package inside.
    if compound.exists("/EncryptedPackage") {
        return Err(WorkbookError::Encrypted.into());
    }
    let name = ["/Workbook", "/Book"].into_iter().find(|name| compound.is_stream(name)).ok_or(WorkbookError::NotAWorkbook)?;
    let mut stream = Vec::new();
    compound
        .open_stream(name)
        .and_then(|mut file| file.read_to_end(&mut stream))
        .map_err(|error| damaged(error.to_string()))?;
    if stream.len() < 4 || u16::from_le_bytes([stream[0], stream[1]]) != 0x0809 {
        // Before Excel 97 (BIFF5 and older): not read.
        return Err(WorkbookError::NotAWorkbook.into());
    }

    let calamine = Xls::new(Cursor::new(shared)).map_err(|error| match error {
        XlsError::Password => WorkbookError::Encrypted,
        error => damaged(error.to_string()),
    })?;

    // The global records, up to the first end of file.
    let mut style_formats = Vec::new();
    let mut codes = HashMap::new();
    let mut offsets = Vec::new();
    let mut active = 0;
    let mut date1904 = false;
    for (kind, data) in Records::new(&stream, 0) {
        match kind {
            EOF => break,
            DATEMODE => date1904 = u16_at(data, 0) == Some(1),
            WINDOW1 => active = u16_at(data, 10).unwrap_or(0) as usize,
            BOUNDSHEET => offsets.push(u32_at(data, 0).unwrap_or(0) as usize),
            XF => style_formats.push(u32::from(u16_at(data, 2).unwrap_or(0))),
            FORMAT => {
                if let (Some(id), Some(code)) = (u16_at(data, 0), unicode_string(data.get(2..).unwrap_or_default(), true)) {
                    codes.insert(u32::from(id), code);
                }
            }
            _ => {}
        }
    }

    let sheets = calamine
        .sheets_metadata()
        .iter()
        .map(|sheet| SheetEntry {
            name: sheet.name.clone(),
            visibility: match sheet.visible {
                SheetVisible::Visible => SheetVisibility::Visible,
                SheetVisible::Hidden => SheetVisibility::Hidden,
                SheetVisible::VeryHidden => SheetVisibility::VeryHidden,
            },
            kind: match sheet.typ {
                SheetType::WorkSheet => SheetKind::Worksheet,
                SheetType::ChartSheet => SheetKind::Chartsheet,
                _ => SheetKind::Other,
            },
            part: None,
        })
        .collect();
    Ok(Opened {
        book: XlsBook { calamine: Mutex::new(calamine), stream, offsets, styles: Styles::new(style_formats, codes) },
        sheets,
        active,
        date1904,
    })
}

fn damaged(detail: String) -> WorkbookError {
    WorkbookError::Damaged { part: "Workbook".to_owned(), position: None, detail }
}

/// Puts a field into its row, adding rows and empty fields before it as needed.
fn put(rows: &mut Vec<Vec<Field>>, row: usize, column: usize, field: Field) {
    if rows.len() <= row {
        rows.resize_with(row + 1, Vec::new);
    }
    let fields = &mut rows[row];
    if fields.len() <= column {
        fields.resize(column + 1, Field::Text(String::new()));
    }
    fields[column] = field;
}

impl XlsBook {
    /// The cells of sheet `index` as Excel shows them, and its layout.
    pub(crate) fn read_sheet(&self, index: usize, name: &str, context: &Context) -> Result<(Vec<Vec<Field>>, SheetLayout)> {
        let values = {
            let mut calamine = self.calamine.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            calamine.worksheet_range(name).map_err(|error| damaged(error.to_string()))?
        };
        let offset = self.offsets.get(index).copied().unwrap_or(usize::MAX);
        let (styles, layout) = self.sheet_records(offset);
        let mut rows: Vec<Vec<Field>> = Vec::new();
        let (top, left) = values.start().unwrap_or((0, 0));
        for (row, column, data) in values.used_cells() {
            let (row, column) = (top + row as u32, left + column as u32);
            let style = styles.get(&(row, column)).copied().unwrap_or(0) as usize;
            let format = context.formats.get(style).unwrap_or(&context.formats[0]);
            let value = match data {
                Data::Int(number) => format.format_number(*number as f64, context.locale, context.date1904),
                Data::Float(number) => format.format_number(*number, context.locale, context.date1904),
                Data::DateTime(date) => format.format_number(date.as_f64(), context.locale, context.date1904),
                Data::String(text) => format.format_text(text),
                Data::Bool(true) => context.locale.true_text.clone(),
                Data::Bool(false) => context.locale.false_text.clone(),
                Data::Error(error) => error.to_string(),
                Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
                Data::Empty => continue,
            };
            if !value.is_empty() {
                put(&mut rows, row as usize, column as usize, Field::Text(value));
            }
        }
        Ok((rows, layout))
    }

    /// Styles of the cells with numbers, and the layout, from the records of the sheet that
    /// begin at `offset`.
    fn sheet_records(&self, offset: usize) -> (HashMap<(u32, u32), u16>, SheetLayout) {
        let mut styles = HashMap::new();
        let mut layout = SheetLayout::default();
        let mut frozen = false;
        let cell = |data: &[u8]| Some((u32::from(u16_at(data, 0)?), u32::from(u16_at(data, 2)?), u16_at(data, 4)?));
        for (kind, data) in Records::new(&self.stream, offset) {
            match kind {
                EOF => break,
                NUMBER | RK | FORMULA => {
                    if let Some((row, column, style)) = cell(data) {
                        styles.insert((row, column), style);
                    }
                }
                MULRK => {
                    let (Some(row), Some(first)) = (u16_at(data, 0), u16_at(data, 2)) else { continue };
                    for (index, entry) in data.get(4..data.len().saturating_sub(2)).unwrap_or_default().chunks_exact(6).enumerate() {
                        styles.insert((u32::from(row), u32::from(first) + index as u32), u16::from_le_bytes([entry[0], entry[1]]));
                    }
                }
                COLINFO => {
                    let (Some(first), Some(last), Some(width), Some(flags)) =
                        (u16_at(data, 0), u16_at(data, 2), u16_at(data, 4), u16_at(data, 8))
                    else {
                        continue;
                    };
                    layout.columns.push(ColumnLayout {
                        first: usize::from(first),
                        last: usize::from(last.min(255)),
                        width: Some(f64::from(width) / 256.0),
                        hidden: flags & 1 != 0,
                    });
                }
                DEFCOLWIDTH => layout.default_column_width = u16_at(data, 0).map(f64::from),
                WINDOW2 => frozen = u16_at(data, 0).is_some_and(|flags| flags & 0x0008 != 0),
                PANE if frozen => {
                    layout.frozen_columns = u16_at(data, 0).unwrap_or(0) as usize;
                    layout.frozen_rows = u16_at(data, 2).unwrap_or(0) as usize;
                }
                MERGEDCELLS => {
                    for range in data.get(2..).unwrap_or_default().chunks_exact(8) {
                        let at = |index: usize| usize::from(u16::from_le_bytes([range[index], range[index + 1]]));
                        layout.merged.push(SheetRange { top: at(0), bottom: at(2), left: at(4), right: at(6) });
                    }
                }
                _ => {}
            }
        }
        (styles, layout)
    }
}

/// BIFF records from `offset`: their type and data. Stops at the end of the stream or at a
/// record that runs past it.
struct Records<'a> {
    stream: &'a [u8],
    position: usize,
}

impl<'a> Records<'a> {
    fn new(stream: &'a [u8], offset: usize) -> Self {
        Self { stream, position: offset }
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = (u16, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let header = self.stream.get(self.position..self.position + 4)?;
        let kind = u16::from_le_bytes([header[0], header[1]]);
        let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        let start = self.position + 4;
        let data = self.stream.get(start..start + length)?;
        self.position = start + length;
        Some((kind, data))
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// An `XLUnicodeString`: a 16-bit length (8-bit when `wide_length` is false), a flag byte,
/// then Latin-1 bytes or UTF-16 units.
fn unicode_string(data: &[u8], wide_length: bool) -> Option<String> {
    let (length, rest) = if wide_length {
        (usize::from(u16_at(data, 0)?), data.get(2..)?)
    } else {
        (usize::from(*data.first()?), data.get(1..)?)
    };
    let (flags, chars) = rest.split_first()?;
    if flags & 1 == 0 {
        Some(chars.get(..length)?.iter().map(|&byte| char::from(byte)).collect())
    } else {
        let units: Vec<u16> = chars.get(..length * 2)?.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        Some(String::from_utf16_lossy(&units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_strings_of_both_widths() {
        assert_eq!(unicode_string(&[3, 0, 0, b'a', b'b', b'c'], true).as_deref(), Some("abc"));
        assert_eq!(unicode_string(&[1, 0, 1, 0x3F, 0x04], true).as_deref(), Some("п"));
        assert_eq!(unicode_string(&[2, 0, b'h', b'i'], false).as_deref(), Some("hi"));
        assert_eq!(unicode_string(&[9, 0, 0, b'a'], true), None);
    }

    #[test]
    fn walks_records_and_stops_at_a_cut_one() {
        let stream = [0x09, 0x08, 2, 0, 1, 2, 0x0A, 0, 0, 0, 0x01, 0x02, 9, 0];
        let records: Vec<_> = Records::new(&stream, 0).collect();
        assert_eq!(records, [(0x0809, &[1u8, 2][..]), (EOF, &[][..])]);
    }
}

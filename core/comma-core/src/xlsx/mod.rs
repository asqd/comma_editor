//! Excel workbooks (XLSX). A sheet opens as a `Document` whose values are what Excel shows:
//! numbers and dates in the cell's number format, shared and formatted strings as text, and
//! cells with a formula keeping it along with its last result.

mod numfmt;
mod package;
mod refs;
mod sheet;
mod styles;
mod xml;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use quick_xml::events::Event;

pub use numfmt::DisplayLocale;
pub use refs::{cell_name, column_letters};

use self::package::Package;
use self::sheet::Context;
use self::styles::Styles;
use self::xml::attribute;
use crate::bytes::{self, Bytes};
use crate::document::Document;
use crate::edit::Progress;
use crate::error::{Error, Result};

/// Whether Excel shows a sheet in its tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetVisibility {
    Visible,
    Hidden,
    /// Hidden so that only a macro can show it again.
    VeryHidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKind {
    /// A sheet of cells.
    Worksheet,
    /// A sheet that is one chart.
    Chartsheet,
    /// A dialog or macro sheet from old versions of Excel.
    Other,
}

/// A sheet as the workbook lists it, in the order of its tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetEntry {
    pub name: String,
    pub visibility: SheetVisibility,
    pub kind: SheetKind,
    /// The part with the sheet's cells.
    part: Option<String>,
}

/// Rows `top..=bottom` and columns `left..=right` of a sheet, from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetRange {
    pub top: usize,
    pub left: usize,
    pub bottom: usize,
    pub right: usize,
}

/// Columns `first..=last` (from 0) as the sheet sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnLayout {
    pub first: usize,
    pub last: usize,
    /// Width in characters of the workbook's default font.
    pub width: Option<f64>,
    pub hidden: bool,
}

/// How a sheet looks in Excel, for the app to show it the same way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetLayout {
    /// Merged cells; the value is in the top left cell of each range.
    pub merged: Vec<SheetRange>,
    pub columns: Vec<ColumnLayout>,
    pub default_column_width: Option<f64>,
    pub frozen_rows: usize,
    pub frozen_columns: usize,
}

/// A sheet read from a workbook. Rows and columns of the document are those of the sheet:
/// row 0 is row 1 in Excel and column 0 is column A, even when they are empty.
pub struct Sheet {
    pub name: String,
    pub document: Document,
    pub layout: SheetLayout,
}

/// An open XLSX workbook. Sheets are read one at a time, when asked for.
pub struct Workbook {
    package: Package,
    sheets: Vec<SheetEntry>,
    strings: Vec<String>,
    styles: Styles,
    date1904: bool,
    /// The tab shown when the workbook was saved.
    active: usize,
}

impl Workbook {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_buffer(bytes::read_file(path.as_ref())?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_buffer(bytes.into())
    }

    fn from_buffer(bytes: Bytes) -> Result<Self> {
        let package = Package::new(bytes)?;
        let workbook_part = package
            .relationships("")?
            .into_iter()
            .find(|relationship| relationship.kind == "officeDocument" && !relationship.external)
            .map(|relationship| relationship.target)
            .filter(|part| package.has_part(part))
            .or_else(|| package.has_part("xl/workbook.xml").then(|| "xl/workbook.xml".to_owned()))
            .ok_or_else(|| Error::Workbook("the file is not an Excel workbook".to_owned()))?;
        let relationships = package.relationships(&workbook_part)?;
        let target = |kind: &str| {
            relationships
                .iter()
                .find(|relationship| relationship.kind == kind && !relationship.external)
                .map(|relationship| relationship.target.clone())
        };

        let text = package.read_text(&workbook_part)?.unwrap_or_default();
        let mut reader = xml::reader(&text);
        let mut sheets = Vec::new();
        let mut date1904 = false;
        let mut active = 0;
        loop {
            match reader.read_event().map_err(|error| xml::error(&workbook_part, &reader, error))? {
                Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                    "workbookPr" => date1904 = xml::flag(&element, "date1904"),
                    "workbookView" => {
                        active = attribute(&element, "activeTab").and_then(|tab| tab.parse().ok()).unwrap_or(0);
                    }
                    "sheet" => {
                        let relationship = attribute(&element, "id")
                            .and_then(|id| relationships.iter().find(|relationship| relationship.id == id));
                        sheets.push(SheetEntry {
                            name: attribute(&element, "name").unwrap_or_default().into_owned(),
                            visibility: match attribute(&element, "state").as_deref() {
                                Some("hidden") => SheetVisibility::Hidden,
                                Some("veryHidden") => SheetVisibility::VeryHidden,
                                _ => SheetVisibility::Visible,
                            },
                            kind: match relationship.map(|relationship| relationship.kind.as_str()) {
                                Some("worksheet") => SheetKind::Worksheet,
                                Some("chartsheet") => SheetKind::Chartsheet,
                                _ => SheetKind::Other,
                            },
                            part: relationship.map(|relationship| relationship.target.clone()),
                        });
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }

        let strings = match target("sharedStrings") {
            Some(part) => match package.read_text(&part)? {
                Some(text) => styles::read_shared_strings(&text, &part)?,
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        let styles = match target("styles") {
            Some(part) => match package.read_text(&part)? {
                Some(text) => Styles::read(&text, &part)?,
                None => Styles::default(),
            },
            None => Styles::default(),
        };
        let active = if sheets.get(active).is_some_and(|sheet| sheet.kind == SheetKind::Worksheet) {
            active
        } else {
            sheets.iter().position(|sheet| sheet.kind == SheetKind::Worksheet && sheet.visibility == SheetVisibility::Visible).unwrap_or(0)
        };
        Ok(Self { package, sheets, strings, styles, date1904, active })
    }

    /// Sheets in the order of their tabs.
    pub fn sheets(&self) -> &[SheetEntry] {
        &self.sheets
    }

    /// The sheet to show first: the one shown when the workbook was saved, if it is a table.
    pub fn active_sheet(&self) -> usize {
        self.active
    }

    /// Whether dates count from 1904, as in workbooks made by old Excel for Mac.
    pub fn uses_1904_dates(&self) -> bool {
        self.date1904
    }

    /// Reads sheet `index`, showing values the way `locale` does.
    pub fn read_sheet(&self, index: usize, locale: &DisplayLocale) -> Result<Sheet> {
        Ok(self.read_sheet_with_progress(index, locale, &Progress::default())?.expect("not cancelled"))
    }

    /// Like `read_sheet`, counting the work in `progress`; `None` if it was cancelled.
    pub fn read_sheet_with_progress(&self, index: usize, locale: &DisplayLocale, progress: &Progress) -> Result<Option<Sheet>> {
        let entry = self.sheets.get(index).ok_or_else(|| Error::Workbook(format!("there is no sheet {}", index + 1)))?;
        if entry.kind != SheetKind::Worksheet {
            return Err(Error::Workbook(format!("“{}” is a chart or macro sheet, not a table", entry.name)));
        }
        let part = entry.part.as_deref().unwrap_or_default();
        let missing = || Error::Workbook(format!("the cells of sheet “{}” are missing", entry.name));
        let (stream, size) = self.package.stream(part)?.ok_or_else(missing)?;
        let mut input = BufReader::with_capacity(1 << 16, stream);
        let formats = self.styles.formats(locale);
        let context = Context { strings: &self.strings, formats: &formats, locale, date1904: self.date1904 };
        let starts_utf16 = matches!(input.fill_buf()?, [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..]);
        let content = if starts_utf16 {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            let text = xml::decode(bytes);
            sheet::read_sheet(&mut xml::reader(&text), text.len() as u64, part, &context, Some(progress))?
        } else {
            sheet::read_sheet(&mut xml::Stream::new(input), size, part, &context, Some(progress))?
        };
        let Some(content) = content else {
            return Ok(None);
        };
        Ok(Some(Sheet {
            name: entry.name.clone(),
            document: Document::from_fields(content.rows),
            layout: content.layout,
        }))
    }
}

// Shared with the app: a workbook is read on one thread and kept behind a lock.
const _: () = {
    const fn sendable<T: Send + Sync>() {}
    sendable::<Workbook>();
    sendable::<Arc<Workbook>>();
};

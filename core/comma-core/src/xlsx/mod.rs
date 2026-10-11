//! Excel workbooks: XLSX, and Excel 97–2003 XLS for reading. A sheet opens as a `Document`
//! whose values are what Excel shows:
//! numbers and dates in the cell's number format, shared and formatted strings as text, and
//! cells with a formula keeping it along with its last result.

mod calc;
mod numfmt;
mod package;
mod refs;
mod save;
mod sheet;
mod structure;
mod styles;
mod xls;
mod xml;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

use quick_xml::events::Event;

pub use calc::{Recalculation, localize_formula};
pub use structure::Axis;
pub(crate) use calc::normalize as normalize_formula;
pub(crate) use refs::shift_formula;
pub use numfmt::DisplayLocale;
pub use refs::{cell_name, column_letters};

use self::package::Package;
use self::sheet::Context;
use self::styles::Styles;
use self::xml::attribute;
use crate::bytes::{self, Bytes};
use crate::document::{Document, SheetSource};
use crate::edit::Progress;
use crate::error::{Result, WorkbookError};

/// The kind of file a workbook was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbookFormat {
    /// Office Open XML: `.xlsx`, `.xlsm`.
    Xlsx,
    /// Excel 97–2003: `.xls`, which is only read.
    Xls,
}

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

/// An open workbook. Sheets are read one at a time, when asked for.
pub struct Workbook {
    source: Source,
    sheets: Vec<SheetEntry>,
    date1904: bool,
    /// The tab shown when the workbook was saved.
    active: usize,
    /// Defined names, charts or pivot tables refer to cells of any sheet.
    refers_to_cells: bool,
    /// Charts or pivot tables do: rows and columns stay where they are.
    locked: bool,
    /// Whether each sheet part holds a formula, found out once when needed.
    formulas: Mutex<HashMap<String, bool>>,
    /// The part with the list of sheets, and the calculation chain if there is one.
    workbook_part: String,
    calc_chain: Option<String>,
    /// Defined names, whose formulas formulas may use by name.
    names: Vec<DefinedName>,
    /// The file asks to compute every formula when opened (`fullCalcOnLoad`): programs that
    /// write workbooks leave results out or write zeros, and Excel computes them on opening.
    computes_on_load: bool,
    /// What computing formulas keeps between edits.
    calc: Mutex<calc::CalcState>,
    /// Shared with every sheet read: a formula was entered somewhere.
    formulas_entered: Arc<std::sync::atomic::AtomicBool>,
}

/// A defined name of a workbook: `Итого_сумма` for `Продажи!$D$13`.
#[derive(Clone, Debug)]
pub(crate) struct DefinedName {
    /// In lower case: names are found without case.
    pub name: String,
    /// The sheet it belongs to, by index; `None` for the whole workbook.
    pub sheet: Option<usize>,
    pub formula: String,
}

enum Source {
    Xlsx { package: Package, strings: Vec<String>, styles: Styles },
    Xls(Box<xls::XlsBook>),
}

impl Workbook {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_buffer(bytes::read_file(path.as_ref())?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_buffer(bytes.into())
    }

    fn from_buffer(bytes: Bytes) -> Result<Self> {
        if bytes.starts_with(&xls::COMPOUND_FILE) {
            let opened = xls::open(bytes)?;
            let active = first_shown(&opened.sheets, opened.active);
            return Ok(Self {
                source: Source::Xls(Box::new(opened.book)),
                sheets: opened.sheets,
                date1904: opened.date1904,
                active,
                refers_to_cells: true,
                locked: true,
                formulas: Mutex::default(),
                workbook_part: String::new(),
                calc_chain: None,
                names: Vec::new(),
                computes_on_load: false,
                calc: Mutex::default(),
                formulas_entered: Arc::default(),
            });
        }
        if !bytes.starts_with(b"PK")
            && let Some(problem) = text_disguised_as_workbook(&bytes)
        {
            return Err(problem.into());
        }
        let package = Package::new(bytes)?;
        let workbook_part = package
            .relationships("")?
            .into_iter()
            .find(|relationship| relationship.kind == "officeDocument" && !relationship.external)
            .map(|relationship| relationship.target)
            .filter(|part| package.has_part(part))
            .or_else(|| package.has_part("xl/workbook.xml").then(|| "xl/workbook.xml".to_owned()))
            .ok_or(WorkbookError::NotAWorkbook)?;
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
        let mut defined_names = false;
        let mut names: Vec<DefinedName> = Vec::new();
        // A defined name being read: its name, the sheet it belongs to, and its formula so far.
        let mut reading: Option<DefinedName> = None;
        let mut computes_on_load = false;
        loop {
            let event = reader.read_event().map_err(|error| xml::error(&workbook_part, &reader, error))?;
            if let Some(name) = reading.as_mut() {
                match &event {
                    Event::End(element) if element.local_name().as_ref() == "definedName" => {
                        names.extend(reading.take());
                        continue;
                    }
                    event => {
                        xml::push_text(&mut name.formula, event);
                        continue;
                    }
                }
            }
            if let Event::Start(element) = &event
                && element.local_name().as_ref() == "definedName"
            {
                defined_names = true;
                reading = Some(DefinedName {
                    name: attribute(element, "name").unwrap_or_default().to_lowercase(),
                    sheet: attribute(element, "localSheetId").and_then(|id| id.parse().ok()),
                    formula: String::new(),
                });
                continue;
            }
            match event {
                Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                    "workbookPr" => date1904 = xml::flag(&element, "date1904"),
                    "definedName" => defined_names = true,
                    "calcPr" => computes_on_load = xml::flag(&element, "fullCalcOnLoad"),
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
        let active = first_shown(&sheets, active);
        let calc_chain = target("calcChain");
        let locked = package.part_names().any(|name| name.starts_with("xl/charts/") || name.starts_with("xl/pivotcache"));
        let refers_to_cells = defined_names || locked;
        Ok(Self {
            source: Source::Xlsx { package, strings, styles },
            sheets,
            date1904,
            active,
            refers_to_cells,
            locked,
            formulas: Mutex::default(),
            workbook_part,
            calc_chain,
            names,
            computes_on_load,
            calc: Mutex::default(),
            formulas_entered: Arc::default(),
        })
    }

    pub fn format(&self) -> WorkbookFormat {
        match self.source {
            Source::Xlsx { .. } => WorkbookFormat::Xlsx,
            Source::Xls(_) => WorkbookFormat::Xls,
        }
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
        let entry = self.sheets.get(index).ok_or(WorkbookError::NoSuchSheet { sheet: index + 1 })?;
        if entry.kind != SheetKind::Worksheet {
            return Err(WorkbookError::NotATable { sheet: entry.name.clone() }.into());
        }
        let (package, strings, styles) = match &self.source {
            Source::Xlsx { package, strings, styles } => (package, strings, styles),
            Source::Xls(book) => {
                let formats = book.styles.formats(locale);
                let context = Context { strings: &[], formats: &formats, locale, date1904: self.date1904 };
                let (rows, layout) = book.read_sheet(index, &entry.name, &context)?;
                let source = SheetSource {
                    rows_fixed: true,
                    structure_locked: true,
                    formulas_moved: Arc::default(),
                    rows: rows.len(),
                    locale: locale.clone(),
                    formulas_entered: Arc::clone(&self.formulas_entered),
                    merged: layout.merged.clone(),
                };
                return Ok(Some(Sheet { name: entry.name.clone(), document: Document::from_sheet(rows, source), layout }));
            }
        };
        let part = entry.part.as_deref().unwrap_or_default();
        let missing = || WorkbookError::MissingCells { sheet: entry.name.clone() };
        let (stream, size) = package.stream(part)?.ok_or_else(missing)?;
        let mut input = BufReader::with_capacity(1 << 16, stream);
        let formats = styles.formats(locale);
        let context = Context { strings, formats: &formats, locale, date1904: self.date1904 };
        let starts_utf16 = matches!(input.fill_buf()?, [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..]);
        let content = if starts_utf16 {
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes)?;
            let text = xml::decode(bytes);
            sheet::read_sheet(&mut xml::reader(&text), text.len() as u64, part, &context, Some(progress), false)?
        } else {
            sheet::read_sheet(&mut xml::Stream::new(input), size, part, &context, Some(progress), false)?
        };
        let Some(content) = content else {
            return Ok(None);
        };
        self.formulas.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(part.to_owned(), content.refers_to_rows);
        let rows_fixed = content.refers_to_rows || self.refers_to_cells || self.other_sheets_have_formulas(part)?;
        let rows = content.rows.len();
        Ok(Some(Sheet {
            name: entry.name.clone(),
            document: Document::from_sheet(
                content.rows,
                SheetSource {
                    rows_fixed,
                    structure_locked: content.locked || self.locked,
                    formulas_moved: Arc::default(),
                    rows,
                    locale: locale.clone(),
                    formulas_entered: Arc::clone(&self.formulas_entered),
                    merged: content.layout.merged.clone(),
                },
            ),
            layout: content.layout,
        }))
    }

    /// Whether a sheet other than the one in `part` has a formula, which may refer to any
    /// sheet. Each part is looked through once.
    fn other_sheets_have_formulas(&self, part: &str) -> Result<bool> {
        if !matches!(self.source, Source::Xlsx { .. }) {
            return Ok(true);
        }
        for other in self.sheets.iter().filter_map(|sheet| sheet.part.as_deref()).filter(|other| *other != part) {
            if self.part_has_formula(other)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether a sheet part holds a formula, found out once.
    fn part_has_formula(&self, part: &str) -> Result<bool> {
        let Source::Xlsx { package, .. } = &self.source else { return Ok(false) };
        if let Some(&has) = self.formulas.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(part) {
            return Ok(has);
        }
        let has = match package.stream(part)? {
            Some((stream, _)) => has_formula(stream)?,
            None => false,
        };
        self.formulas.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(part.to_owned(), has);
        Ok(has)
    }

    /// The sheets to read and compute as soon as the workbook opens: in a file that asks to
    /// compute every formula on opening, the sheets with formulas, since the results it stores
    /// cannot be trusted and formulas read one another across sheets. Empty for other files.
    pub fn sheets_computed_on_open(&self) -> Result<Vec<usize>> {
        if !self.computes_on_load {
            return Ok(Vec::new());
        }
        let mut sheets = Vec::new();
        for (index, sheet) in self.sheets.iter().enumerate() {
            if sheet.kind == SheetKind::Worksheet
                && let Some(part) = sheet.part.as_deref()
                && self.part_has_formula(part)?
            {
                sheets.push(index);
            }
        }
        Ok(sheets)
    }
}

/// Web pages and XML spreadsheets that exports save as `.xls`, told by how they begin.
fn text_disguised_as_workbook(bytes: &[u8]) -> Option<WorkbookError> {
    let start = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_lowercase();
    let start = start.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if !start.starts_with('<') {
        return None;
    }
    if start.contains("urn:schemas-microsoft-com:office:spreadsheet") {
        Some(WorkbookError::Xml2003)
    } else if ["<html", "<!doctype html", "<table", "<meta"].iter().any(|tag| start.contains(tag)) {
        Some(WorkbookError::WebPage)
    } else {
        None
    }
}

/// Whether XML holds a formula element, `<f>` or `<f …>` with any namespace prefix.
fn has_formula(mut input: impl Read) -> Result<bool> {
    let mut buffer = vec![0; 1 << 16];
    // The end of the previous chunk, so a tag cut between chunks is still found.
    let mut carry = 0;
    loop {
        let read = input.read(&mut buffer[carry..])?;
        if read == 0 {
            return Ok(false);
        }
        let filled = carry + read;
        let chunk = &buffer[..filled];
        let found = memchr::memchr_iter(b'<', chunk).any(|at| is_formula_tag(&chunk[at + 1..]));
        if found {
            return Ok(true);
        }
        carry = filled.min(16);
        buffer.copy_within(filled - carry..filled, 0);
    }
}

/// Whether the text after a `<` starts a formula element: `f`, or `x:f` with a prefix.
fn is_formula_tag(tag: &[u8]) -> bool {
    let end = tag.iter().position(|b| !(b.is_ascii_alphanumeric() || *b == b':')).unwrap_or(tag.len());
    let name = &tag[..end];
    let local = name.rsplit(|&b| b == b':').next().unwrap_or(name);
    local == b"f" && matches!(tag.get(end), Some(b'>' | b' ' | b'/'))
}

/// The tab saved as shown if it is a table, or else the first visible table.
fn first_shown(sheets: &[SheetEntry], saved: usize) -> usize {
    if sheets.get(saved).is_some_and(|sheet| sheet.kind == SheetKind::Worksheet) {
        return saved;
    }
    sheets
        .iter()
        .position(|sheet| sheet.kind == SheetKind::Worksheet && sheet.visibility == SheetVisibility::Visible)
        .unwrap_or(0)
}

// Shared with the app: a workbook is read on one thread and kept behind a lock.
const _: () = {
    const fn sendable<T: Send + Sync>() {}
    sendable::<Workbook>();
    sendable::<Arc<Workbook>>();
};

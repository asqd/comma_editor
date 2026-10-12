//! UniFFI bindings of `comma-core` for the macOS app.
//!
//! Indices cross the boundary as `u64`; rows include the header row, if there is one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use comma_core as core;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// `code` is the system's error number, which the app turns into the system's own words.
    #[error("{message}")]
    Io { message: String, code: Option<i32> },
    #[error("{message}")]
    OutOfRange { message: String },
    #[error("the value in row {row}, column {column} cannot be saved in {encoding}")]
    Unmappable {
        row: u64,
        column: u64,
        encoding: String,
    },
    #[error("the last column cannot be deleted")]
    LastColumn,
    #[error("{message}")]
    InvalidPattern { message: String },
    #[error("{delimiter} cannot be used as a delimiter")]
    InvalidDelimiter { delimiter: String },
    #[error("{message}")]
    Export { message: String },
    /// An Excel sheet holds at most `limit` data rows.
    #[error("Excel sheets hold at most {limit} rows; this table has {rows}")]
    ExcelRowLimit { rows: u64, limit: u64 },
    #[error("{problem:?}")]
    Formula { problem: FormulaProblem },
    /// The file is still being indexed; it cannot be changed or saved yet.
    #[error("the file is still being read")]
    Busy,
    /// A change runs in the background; others wait until it ends.
    #[error("another change is being made")]
    Editing,
    /// A step of a recipe failed; `step` counts from 0. `column` is a column the file lacks.
    #[error("step {step}: {message}")]
    Recipe { step: u64, column: Option<String>, message: String },
    #[error("{message}")]
    InvalidRecipe { message: String },
    /// An Excel workbook that cannot be read, and why.
    #[error("{problem:?}")]
    Workbook { problem: WorkbookProblem },
    /// A change to rows or columns a sheet of a workbook cannot take and still be saved.
    #[error("the rows or columns of this sheet cannot change")]
    SheetStructure { change: SheetChange },
    /// The cell is hidden by a merged area; its value is in the area's first cell, by row of
    /// the file and column.
    #[error("the cell is inside a merged area")]
    MergedCell { row: u64, column: u64 },
}

/// Why an Excel workbook or one of its sheets cannot be read.
#[derive(Debug, uniffi::Enum)]
pub enum WorkbookProblem {
    NotAWorkbook,
    /// A web page saved with a workbook's extension, as many exports are.
    WebPage,
    /// An Excel 2003 XML spreadsheet saved with a workbook's extension.
    Xml2003,
    /// The workbook is protected with a password.
    Encrypted,
    /// A part of the file cannot be unpacked, or its XML is broken at byte `position`.
    Damaged { part: String, position: Option<u64>, detail: String },
    /// `sheet` counts from 1.
    NoSuchSheet { sheet: u64 },
    NotATable { sheet: String },
    MissingCells { sheet: String },
    /// Excel 97–2003 workbooks are read, not saved.
    ReadOnly,
}

impl From<core::WorkbookError> for WorkbookProblem {
    fn from(error: core::WorkbookError) -> Self {
        match error {
            core::WorkbookError::NotAWorkbook => Self::NotAWorkbook,
            core::WorkbookError::WebPage => Self::WebPage,
            core::WorkbookError::Xml2003 => Self::Xml2003,
            core::WorkbookError::Encrypted => Self::Encrypted,
            core::WorkbookError::Damaged { part, position, detail } => Self::Damaged { part, position, detail },
            core::WorkbookError::NoSuchSheet { sheet } => Self::NoSuchSheet { sheet: sheet as u64 },
            core::WorkbookError::NotATable { sheet } => Self::NotATable { sheet },
            core::WorkbookError::MissingCells { sheet } => Self::MissingCells { sheet },
            core::WorkbookError::ReadOnly => Self::ReadOnly,
        }
    }
}

impl From<core::Error> for CoreError {
    fn from(error: core::Error) -> Self {
        match error {
            core::Error::Io(error) => Self::Io {
                message: error.to_string(),
                code: error.raw_os_error(),
            },
            core::Error::RowOutOfRange { .. } | core::Error::ColumnOutOfRange { .. } => {
                Self::OutOfRange {
                    message: error.to_string(),
                }
            }
            core::Error::Unmappable {
                row,
                column,
                encoding,
            } => Self::Unmappable {
                row: row as u64,
                column: column as u64,
                encoding: encoding.to_owned(),
            },
            core::Error::LastColumn => Self::LastColumn,
            core::Error::InvalidPattern(message) => Self::InvalidPattern { message },
            core::Error::InvalidDelimiter(delimiter) => Self::InvalidDelimiter {
                delimiter: delimiter.to_string(),
            },
            core::Error::Export(message) => Self::Export { message },
            core::Error::TooManyRowsForExcel { rows, limit } => Self::ExcelRowLimit { rows: rows as u64, limit: limit as u64 },
            core::Error::Formula(error) => Self::Formula { problem: error.into() },
            core::Error::UnknownColumn(ref column) => Self::Recipe { step: 0, column: Some(column.clone()), message: error.to_string() },
            core::Error::InvalidRecipe(message) => Self::InvalidRecipe { message },
            core::Error::Workbook(problem) => Self::Workbook { problem: problem.into() },
            core::Error::SheetStructure(change) => Self::SheetStructure { change: change.into() },
            core::Error::MergedCell { row, column } => Self::MergedCell { row: row as u64, column: column as u64 },
            core::Error::Recipe { step, source } => {
                let column = match source.as_ref() {
                    core::Error::UnknownColumn(column) => Some(column.clone()),
                    _ => None,
                };
                Self::Recipe { step: step as u64, column, message: source.to_string() }
            }
        }
    }
}

/// What splitting a column of a sheet did: the sheets changed and the columns added.
#[derive(uniffi::Record)]
pub struct ColumnSplit {
    pub sheets: Vec<u64>,
    pub count: u64,
}

/// Why rows or columns of a sheet cannot change: see `comma_core::SheetStructure`.
#[derive(Debug, uniffi::Enum)]
pub enum SheetChange {
    Rows,
    Columns,
    PivotTable,
    TableRowsMoved,
    TableEdges,
    TableColumns,
    TableColumnInUse,
    MergedArea,
}

impl From<core::SheetStructure> for SheetChange {
    fn from(change: core::SheetStructure) -> Self {
        match change {
            core::SheetStructure::RowsFixed => Self::Rows,
            core::SheetStructure::ColumnsFixed => Self::Columns,
            core::SheetStructure::PivotTable => Self::PivotTable,
            core::SheetStructure::TableRowsMoved => Self::TableRowsMoved,
            core::SheetStructure::TableEdges => Self::TableEdges,
            core::SheetStructure::TableColumns => Self::TableColumns,
            core::SheetStructure::TableColumnInUse => Self::TableColumnInUse,
            core::SheetStructure::MergedArea => Self::MergedArea,
        }
    }
}

#[derive(Debug, uniffi::Enum)]
pub enum FormulaErrorKind {
    UnexpectedEnd,
    UnexpectedText,
    UnclosedText,
    UnclosedColumn,
    UnknownColumn,
    UnknownFunction,
    WrongArgumentCount,
    NotANumber,
    NotADate,
    DivisionByZero,
}

/// What is wrong with a formula or with its value in one row. `position` is a character
/// offset in the formula, for syntax errors.
#[derive(Debug, uniffi::Record)]
pub struct FormulaProblem {
    pub kind: FormulaErrorKind,
    pub position: u64,
    pub detail: String,
}

impl From<core::FormulaError> for FormulaProblem {
    fn from(error: core::FormulaError) -> Self {
        Self {
            kind: match error.kind {
                core::FormulaErrorKind::UnexpectedEnd => FormulaErrorKind::UnexpectedEnd,
                core::FormulaErrorKind::UnexpectedText => FormulaErrorKind::UnexpectedText,
                core::FormulaErrorKind::UnclosedText => FormulaErrorKind::UnclosedText,
                core::FormulaErrorKind::UnclosedColumn => FormulaErrorKind::UnclosedColumn,
                core::FormulaErrorKind::UnknownColumn => FormulaErrorKind::UnknownColumn,
                core::FormulaErrorKind::UnknownFunction => FormulaErrorKind::UnknownFunction,
                core::FormulaErrorKind::WrongArgumentCount => FormulaErrorKind::WrongArgumentCount,
                core::FormulaErrorKind::NotANumber => FormulaErrorKind::NotANumber,
                core::FormulaErrorKind::NotADate => FormulaErrorKind::NotADate,
                core::FormulaErrorKind::DivisionByZero => FormulaErrorKind::DivisionByZero,
            },
            position: error.position as u64,
            detail: error.detail,
        }
    }
}

/// The value of a formula in one row, or why it failed.
#[derive(uniffi::Record)]
pub struct FormulaOutcome {
    pub value: Option<String>,
    pub problem: Option<FormulaProblem>,
}

#[derive(uniffi::Record)]
pub struct FormulaReport {
    pub rows: u64,
    pub errors: u64,
    /// The first failing row of the file, and why.
    pub first_error_row: Option<u64>,
    pub first_error: Option<FormulaProblem>,
}

impl From<core::FormulaReport> for FormulaReport {
    fn from(report: core::FormulaReport) -> Self {
        let (row, problem) = report.first_error.map(|(row, error)| (row as u64, error.into())).unzip();
        Self {
            rows: report.rows as u64,
            errors: report.errors as u64,
            first_error_row: row,
            first_error: problem,
        }
    }
}

/// Names of the functions formulas can call.
#[uniffi::export]
pub fn formula_functions() -> Vec<String> {
    core::FUNCTIONS.iter().map(|(name, ..)| (*name).to_owned()).collect()
}

#[derive(uniffi::Enum)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

#[derive(uniffi::Enum)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1251,
    Koi8R,
    Windows1252,
    Windows1250,
    Windows1253,
    Windows1254,
    Windows1257,
    Koi8U,
    Ibm866,
}

impl From<core::TextEncoding> for TextEncoding {
    fn from(encoding: core::TextEncoding) -> Self {
        match encoding {
            core::TextEncoding::Utf8 => Self::Utf8,
            core::TextEncoding::Utf16Le => Self::Utf16Le,
            core::TextEncoding::Utf16Be => Self::Utf16Be,
            core::TextEncoding::Windows1251 => Self::Windows1251,
            core::TextEncoding::Koi8R => Self::Koi8R,
            core::TextEncoding::Windows1252 => Self::Windows1252,
            core::TextEncoding::Windows1250 => Self::Windows1250,
            core::TextEncoding::Windows1253 => Self::Windows1253,
            core::TextEncoding::Windows1254 => Self::Windows1254,
            core::TextEncoding::Windows1257 => Self::Windows1257,
            core::TextEncoding::Koi8U => Self::Koi8U,
            core::TextEncoding::Ibm866 => Self::Ibm866,
        }
    }
}

impl From<TextEncoding> for core::TextEncoding {
    fn from(encoding: TextEncoding) -> Self {
        match encoding {
            TextEncoding::Utf8 => Self::Utf8,
            TextEncoding::Utf16Le => Self::Utf16Le,
            TextEncoding::Utf16Be => Self::Utf16Be,
            TextEncoding::Windows1251 => Self::Windows1251,
            TextEncoding::Koi8R => Self::Koi8R,
            TextEncoding::Windows1252 => Self::Windows1252,
            TextEncoding::Windows1250 => Self::Windows1250,
            TextEncoding::Windows1253 => Self::Windows1253,
            TextEncoding::Windows1254 => Self::Windows1254,
            TextEncoding::Windows1257 => Self::Windows1257,
            TextEncoding::Koi8U => Self::Koi8U,
            TextEncoding::Ibm866 => Self::Ibm866,
        }
    }
}

/// One ASCII character as a delimiter byte.
fn delimiter_byte(delimiter: &str) -> Result<u8, CoreError> {
    let mut chars = delimiter.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() && c != '"' && c != '\n' && c != '\r' => Ok(c as u8),
        _ => Err(CoreError::InvalidDelimiter {
            delimiter: delimiter.to_string(),
        }),
    }
}

fn open_options(encoding: Option<TextEncoding>, delimiter: Option<String>) -> Result<core::OpenOptions, CoreError> {
    Ok(core::OpenOptions {
        encoding: encoding.map(Into::into),
        delimiter: delimiter.as_deref().map(delimiter_byte).transpose()?,
    })
}

/// The first rows of a file as they would open with some options.
#[derive(uniffi::Record)]
pub struct FilePreview {
    pub encoding: TextEncoding,
    pub delimiter: String,
    pub has_header: bool,
    pub rows: Vec<Vec<String>>,
    pub column_count: u64,
    pub detection_confident: bool,
}

/// How the start of a file opens with the given encoding and delimiter (`None` detects them).
#[uniffi::export]
pub fn preview_file(
    path: String,
    encoding: Option<TextEncoding>,
    delimiter: Option<String>,
    rows: u64,
) -> Result<FilePreview, CoreError> {
    let preview = core::Document::preview_file(path, &open_options(encoding, delimiter)?, rows as usize, 1 << 20)?;
    Ok(FilePreview {
        encoding: preview.encoding.into(),
        delimiter: (preview.delimiter as char).to_string(),
        has_header: preview.has_header,
        rows: preview.rows,
        column_count: preview.column_count as u64,
        detection_confident: preview.detection_confident,
    })
}

/// A formula of a workbook as Excel shows it where numbers have a decimal comma (`;` between
/// arguments, `1,5`), with Russian function names when `russian_names`.
#[uniffi::export]
pub fn localize_formula(formula: String, russian_names: bool) -> String {
    core::localize_formula(&formula, russian_names)
}

/// The text read in the right encoding, if it looks garbled.
#[uniffi::export]
pub fn fix_garbled(text: String) -> Option<String> {
    core::fix_garbled(&text)
}

/// Encoding, BOM, delimiter and line endings a file is written with.
#[derive(uniffi::Record)]
pub struct SaveFormat {
    pub encoding: TextEncoding,
    pub bom: bool,
    /// One ASCII character.
    pub delimiter: String,
    pub line_ending: LineEnding,
}

impl From<core::SaveFormat> for SaveFormat {
    fn from(format: core::SaveFormat) -> Self {
        Self {
            encoding: match format.encoding {
                core::TextEncoding::Utf8 => TextEncoding::Utf8,
                core::TextEncoding::Utf16Le => TextEncoding::Utf16Le,
                core::TextEncoding::Utf16Be => TextEncoding::Utf16Be,
                core::TextEncoding::Windows1251 => TextEncoding::Windows1251,
                core::TextEncoding::Koi8R => TextEncoding::Koi8R,
                core::TextEncoding::Windows1252 => TextEncoding::Windows1252,
                core::TextEncoding::Windows1250 => TextEncoding::Windows1250,
                core::TextEncoding::Windows1253 => TextEncoding::Windows1253,
                core::TextEncoding::Windows1254 => TextEncoding::Windows1254,
                core::TextEncoding::Windows1257 => TextEncoding::Windows1257,
                core::TextEncoding::Koi8U => TextEncoding::Koi8U,
                core::TextEncoding::Ibm866 => TextEncoding::Ibm866,
            },
            bom: format.bom,
            delimiter: (format.delimiter as char).to_string(),
            line_ending: format.line_ending.into(),
        }
    }
}

impl TryFrom<SaveFormat> for core::SaveFormat {
    type Error = CoreError;

    fn try_from(format: SaveFormat) -> Result<Self, CoreError> {
        let mut chars = format.delimiter.chars();
        let delimiter = match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii() => c as u8,
            _ => {
                return Err(CoreError::InvalidDelimiter {
                    delimiter: format.delimiter.clone(),
                });
            }
        };
        Ok(Self {
            encoding: match format.encoding {
                TextEncoding::Utf8 => core::TextEncoding::Utf8,
                TextEncoding::Utf16Le => core::TextEncoding::Utf16Le,
                TextEncoding::Utf16Be => core::TextEncoding::Utf16Be,
                TextEncoding::Windows1251 => core::TextEncoding::Windows1251,
                TextEncoding::Koi8R => core::TextEncoding::Koi8R,
                TextEncoding::Windows1252 => core::TextEncoding::Windows1252,
                TextEncoding::Windows1250 => core::TextEncoding::Windows1250,
                TextEncoding::Windows1253 => core::TextEncoding::Windows1253,
                TextEncoding::Windows1254 => core::TextEncoding::Windows1254,
                TextEncoding::Windows1257 => core::TextEncoding::Windows1257,
                TextEncoding::Koi8U => core::TextEncoding::Koi8U,
                TextEncoding::Ibm866 => core::TextEncoding::Ibm866,
            },
            bom: format.bom,
            delimiter,
            line_ending: match format.line_ending {
                LineEnding::Lf => core::LineEnding::Lf,
                LineEnding::CrLf => core::LineEnding::CrLf,
                LineEnding::Cr => core::LineEnding::Cr,
            },
        })
    }
}

impl From<core::LineEnding> for LineEnding {
    fn from(line_ending: core::LineEnding) -> Self {
        match line_ending {
            core::LineEnding::Lf => Self::Lf,
            core::LineEnding::CrLf => Self::CrLf,
            core::LineEnding::Cr => Self::Cr,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum QuoteStyle {
    Minimal,
    Always,
}

#[derive(uniffi::Record)]
pub struct DocumentInfo {
    /// Display name, e.g. "UTF-8" or "Windows-1251".
    pub encoding: String,
    pub has_bom: bool,
    pub delimiter: String,
    pub line_ending: LineEnding,
    pub quote_style: QuoteStyle,
    pub has_header: bool,
    pub trailing_newline: bool,
    pub row_count: u64,
    pub column_count: u64,
    /// Data rows shown by the current sort and filters.
    pub visible_row_count: u64,
    pub unterminated_quote: bool,
    pub text_encoding: TextEncoding,
    /// The encoding and delimiter were detected with certainty, or chosen when opening.
    pub detection_confident: bool,
    /// A sheet of an Excel workbook: rows end at their last value, cells may hold formulas,
    /// and columns cannot be inserted, deleted or moved.
    pub is_sheet: bool,
    /// Rows of the sheet cannot be inserted, deleted or moved on their own, only added at the
    /// end; inserting and deleting them through the workbook moves references to them.
    pub sheet_rows_fixed: bool,
    /// Rows and columns of the sheet stay where they are: tables, drawings, comments, charts or
    /// pivot tables are anchored to them.
    pub sheet_structure_locked: bool,
}

#[derive(uniffi::Record)]
pub struct SortKey {
    pub column: u64,
    pub ascending: bool,
}

#[derive(uniffi::Enum)]
pub enum FilterOp {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    EndsWith,
    Matches,
    IsEmpty,
    IsNotEmpty,
    GreaterThan,
    LessThan,
}

#[derive(uniffi::Record)]
pub struct FilterRule {
    /// `None` means any column.
    pub column: Option<u64>,
    pub op: FilterOp,
    pub value: String,
    pub case_sensitive: bool,
}

#[derive(uniffi::Record)]
pub struct ViewSpec {
    pub sort: Vec<SortKey>,
    pub filters: Vec<FilterRule>,
}

#[derive(uniffi::Record)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_cell: bool,
    pub regex: bool,
    pub column: Option<u64>,
    /// Search these visible cells only.
    pub range: Option<CellRange>,
}

/// Visible rows `top..=bottom` and columns `left..=right`.
#[derive(uniffi::Record)]
pub struct CellRange {
    pub top: u64,
    pub left: u64,
    pub bottom: u64,
    pub right: u64,
}

impl From<CellRange> for core::CellRange {
    fn from(range: CellRange) -> Self {
        Self {
            top: range.top as usize,
            left: range.left as usize,
            bottom: range.bottom as usize,
            right: range.right as usize,
        }
    }
}

impl From<core::CellRange> for CellRange {
    fn from(range: core::CellRange) -> Self {
        Self {
            top: range.top as u64,
            left: range.left as u64,
            bottom: range.bottom as u64,
            right: range.right as u64,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum ValueKind {
    Empty,
    Integer,
    Decimal,
    Date,
    Boolean,
    Text,
}

impl From<core::ValueKind> for ValueKind {
    fn from(kind: core::ValueKind) -> Self {
        match kind {
            core::ValueKind::Empty => Self::Empty,
            core::ValueKind::Integer => Self::Integer,
            core::ValueKind::Decimal => Self::Decimal,
            core::ValueKind::Date => Self::Date,
            core::ValueKind::Boolean => Self::Boolean,
            core::ValueKind::Text => Self::Text,
        }
    }
}

impl From<ValueKind> for core::ValueKind {
    fn from(kind: ValueKind) -> Self {
        match kind {
            ValueKind::Empty => Self::Empty,
            ValueKind::Integer => Self::Integer,
            ValueKind::Decimal => Self::Decimal,
            ValueKind::Date => Self::Date,
            ValueKind::Boolean => Self::Boolean,
            ValueKind::Text => Self::Text,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum Issue {
    Mismatch,
    Padding,
}

#[derive(uniffi::Record)]
pub struct ValueCount {
    pub value: String,
    pub count: u64,
}

/// Statistics of one column over the visible rows.
#[derive(uniffi::Record)]
pub struct ColumnProfile {
    pub kind: ValueKind,
    pub total: u64,
    pub empty: u64,
    pub unique: u64,
    pub mismatched: u64,
    pub padded: u64,
    pub min: Option<String>,
    pub max: Option<String>,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub min_length: u64,
    pub max_length: u64,
    pub top_values: Vec<ValueCount>,
    pub histogram: Vec<u64>,
}

#[derive(uniffi::Enum)]
pub enum ProblemKind {
    UnclosedQuote,
    StrayQuote,
    MissingFields,
    ExtraFields,
}

impl From<core::ProblemKind> for ProblemKind {
    fn from(kind: core::ProblemKind) -> Self {
        match kind {
            core::ProblemKind::UnclosedQuote => Self::UnclosedQuote,
            core::ProblemKind::StrayQuote => Self::StrayQuote,
            core::ProblemKind::MissingFields => Self::MissingFields,
            core::ProblemKind::ExtraFields => Self::ExtraFields,
        }
    }
}

impl From<ProblemKind> for core::ProblemKind {
    fn from(kind: ProblemKind) -> Self {
        match kind {
            ProblemKind::UnclosedQuote => Self::UnclosedQuote,
            ProblemKind::StrayQuote => Self::StrayQuote,
            ProblemKind::MissingFields => Self::MissingFields,
            ProblemKind::ExtraFields => Self::ExtraFields,
        }
    }
}

/// A broken row; `row` counts the header row.
#[derive(uniffi::Record)]
pub struct Problem {
    pub row: u64,
    pub kind: ProblemKind,
    pub fields: u64,
    /// Lines of the file the row takes.
    pub lines: u64,
}

#[derive(uniffi::Record)]
pub struct ProblemCount {
    pub kind: ProblemKind,
    pub count: u64,
}

#[derive(uniffi::Record)]
pub struct RepairReport {
    /// The number of fields most rows have.
    pub expected_fields: u64,
    /// The first problems in file order.
    pub problems: Vec<Problem>,
    /// All problems of each kind.
    pub counts: Vec<ProblemCount>,
    /// Values in the first rows that look like text read in the wrong encoding.
    pub garbled_sample: u64,
}

#[derive(uniffi::Enum)]
pub enum RepairFix {
    QuoteAsText,
    RemoveQuote,
    PadFields,
    JoinNext,
    MergeFields { column: u64 },
    DropEmptyFields,
    DeleteRow,
}

impl From<core::Fix> for RepairFix {
    fn from(fix: core::Fix) -> Self {
        match fix {
            core::Fix::QuoteAsText => Self::QuoteAsText,
            core::Fix::RemoveQuote => Self::RemoveQuote,
            core::Fix::PadFields => Self::PadFields,
            core::Fix::JoinNext => Self::JoinNext,
            core::Fix::MergeFields { column } => Self::MergeFields { column: column as u64 },
            core::Fix::DropEmptyFields => Self::DropEmptyFields,
            core::Fix::DeleteRow => Self::DeleteRow,
        }
    }
}

impl From<RepairFix> for core::Fix {
    fn from(fix: RepairFix) -> Self {
        match fix {
            RepairFix::QuoteAsText => Self::QuoteAsText,
            RepairFix::RemoveQuote => Self::RemoveQuote,
            RepairFix::PadFields => Self::PadFields,
            RepairFix::JoinNext => Self::JoinNext,
            RepairFix::MergeFields { column } => Self::MergeFields { column: column as usize },
            RepairFix::DropEmptyFields => Self::DropEmptyFields,
            RepairFix::DeleteRow => Self::DeleteRow,
        }
    }
}

/// A fix with the first rows it would produce.
#[derive(uniffi::Record)]
pub struct RepairOption {
    pub fix: RepairFix,
    pub rows: Vec<Vec<String>>,
    pub row_count: u64,
}

/// The kind of one value, as column profiles classify it.
#[uniffi::export]
pub fn classify_value(value: String) -> ValueKind {
    core::classify(&value).into()
}

/// Whether a value of `value` kind belongs in a column of `column` kind.
#[uniffi::export]
pub fn kind_fits(value: ValueKind, column: ValueKind) -> bool {
    core::ValueKind::from(value).fits(column.into())
}

#[derive(uniffi::Enum)]
pub enum ExportFormat {
    Json,
    Markdown,
    Html,
    Sql,
    Xlsx,
}

impl From<ExportFormat> for core::ExportFormat {
    fn from(format: ExportFormat) -> Self {
        match format {
            ExportFormat::Json => Self::Json,
            ExportFormat::Markdown => Self::Markdown,
            ExportFormat::Html => Self::Html,
            ExportFormat::Sql => Self::Sql,
            ExportFormat::Xlsx => Self::Xlsx,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum TextTransform {
    Trim,
    CollapseSpaces,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl From<TextTransform> for core::TextTransform {
    fn from(transform: TextTransform) -> Self {
        match transform {
            TextTransform::Trim => Self::Trim,
            TextTransform::CollapseSpaces => Self::CollapseSpaces,
            TextTransform::Uppercase => Self::Uppercase,
            TextTransform::Lowercase => Self::Lowercase,
            TextTransform::Capitalize => Self::Capitalize,
        }
    }
}

#[derive(uniffi::Record)]
pub struct RangeStats {
    pub cells: u64,
    pub filled: u64,
    pub numbers: u64,
    pub sum: f64,
}

/// Cells from clipboard text: tab-separated text is split into cells, other text gives one
/// cell per line.
#[uniffi::export]
pub fn parse_pasted_text(text: String) -> Vec<Vec<String>> {
    core::parse_pasted_text(&text)
}

/// A visible row and a column.
#[derive(uniffi::Record)]
pub struct CellPosition {
    pub row: u64,
    pub column: u64,
}

impl From<SearchQuery> for core::SearchQuery {
    fn from(query: SearchQuery) -> Self {
        Self {
            text: query.text,
            case_sensitive: query.case_sensitive,
            whole_cell: query.whole_cell,
            regex: query.regex,
            column: query.column.map(|column| column as usize),
            range: query.range.map(Into::into),
        }
    }
}

impl From<ViewSpec> for core::ViewSpec {
    fn from(spec: ViewSpec) -> Self {
        Self {
            sort: spec
                .sort
                .into_iter()
                .map(|key| core::SortKey {
                    column: key.column as usize,
                    ascending: key.ascending,
                })
                .collect(),
            filters: spec
                .filters
                .into_iter()
                .map(|rule| core::Filter {
                    column: rule.column.map(|column| column as usize),
                    op: match rule.op {
                        FilterOp::Contains => core::FilterOp::Contains,
                        FilterOp::NotContains => core::FilterOp::NotContains,
                        FilterOp::Equals => core::FilterOp::Equals,
                        FilterOp::NotEquals => core::FilterOp::NotEquals,
                        FilterOp::StartsWith => core::FilterOp::StartsWith,
                        FilterOp::EndsWith => core::FilterOp::EndsWith,
                        FilterOp::Matches => core::FilterOp::Matches,
                        FilterOp::IsEmpty => core::FilterOp::IsEmpty,
                        FilterOp::IsNotEmpty => core::FilterOp::IsNotEmpty,
                        FilterOp::GreaterThan => core::FilterOp::GreaterThan,
                        FilterOp::LessThan => core::FilterOp::LessThan,
                    },
                    value: rule.value,
                    case_sensitive: rule.case_sensitive,
                })
                .collect(),
        }
    }
}

impl From<&core::ViewSpec> for ViewSpec {
    fn from(spec: &core::ViewSpec) -> Self {
        Self {
            sort: spec
                .sort
                .iter()
                .map(|key| SortKey {
                    column: key.column as u64,
                    ascending: key.ascending,
                })
                .collect(),
            filters: spec
                .filters
                .iter()
                .map(|filter| FilterRule {
                    column: filter.column.map(|column| column as u64),
                    op: match filter.op {
                        core::FilterOp::Contains => FilterOp::Contains,
                        core::FilterOp::NotContains => FilterOp::NotContains,
                        core::FilterOp::Equals => FilterOp::Equals,
                        core::FilterOp::NotEquals => FilterOp::NotEquals,
                        core::FilterOp::StartsWith => FilterOp::StartsWith,
                        core::FilterOp::EndsWith => FilterOp::EndsWith,
                        core::FilterOp::Matches => FilterOp::Matches,
                        core::FilterOp::IsEmpty => FilterOp::IsEmpty,
                        core::FilterOp::IsNotEmpty => FilterOp::IsNotEmpty,
                        core::FilterOp::GreaterThan => FilterOp::GreaterThan,
                        core::FilterOp::LessThan => FilterOp::LessThan,
                    },
                    value: filter.value.clone(),
                    case_sensitive: filter.case_sensitive,
                })
                .collect(),
        }
    }
}

#[derive(uniffi::Object)]
pub struct CoreDocument {
    inner: Mutex<core::Document>,
    /// Indexes the rest of a large file; a separate lock, so indexing does not block reading.
    indexer: Mutex<Option<core::Indexer>>,
    /// An edit job runs, from `begin_edit` to `finish_edit`.
    editing: AtomicBool,
    /// Steps recorded for a recipe, while recording.
    recorder: Mutex<Option<Recorder>>,
}

/// Recipe steps recorded so far, each with the change it repeats, so undo and redo take the
/// step out and put it back.
#[derive(Default)]
struct Recorder {
    steps: Vec<(core::RecipeStep, u64)>,
    undone: Vec<(core::RecipeStep, u64)>,
}

/// How far indexing of a large file has come.
#[derive(uniffi::Record)]
pub struct IndexProgress {
    pub done: bool,
    pub indexed_bytes: u64,
    pub total_bytes: u64,
}

/// Sorting and filtering computed off the document; see `CoreDocument::begin_view`.
#[derive(uniffi::Object)]
pub struct ViewJob {
    task: Mutex<Option<core::ViewTask>>,
    cancel: AtomicBool,
}

#[uniffi::export]
impl ViewJob {
    /// Computes the rows to show; call off the main thread. Returns `false` when cancelled.
    pub fn run(&self) -> bool {
        let mut task = self.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        task.as_mut().is_some_and(|task| task.run(&self.cancel))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A change to many rows; see `core::Edit`.
#[derive(uniffi::Enum)]
pub enum EditRequest {
    ReplaceAll { query: SearchQuery, replacement: String },
    Transform { range: CellRange, transform: TextTransform },
    FillEmpty { range: CellRange, from_above: bool, value: String },
    /// A range of visible cells, or the whole file with the header when `None`.
    FixGarbled { range: Option<CellRange> },
    AddFormulaColumn { formula: String, name: String, at: u64 },
    FillWithFormula { formula: String, column: u64 },
    SplitColumn { column: u64, separator: String, regex: bool },
    MergeColumns { columns: Vec<u64>, separator: String },
    RemoveDuplicates { columns: Vec<u64>, normalize: bool },
    ApplyRecipe { recipe: Arc<CoreRecipe> },
}

impl From<EditRequest> for core::Edit {
    fn from(request: EditRequest) -> Self {
        let columns = |columns: Vec<u64>| columns.into_iter().map(|column| column as usize).collect();
        match request {
            EditRequest::ReplaceAll { query, replacement } => Self::ReplaceAll { query: query.into(), replacement },
            EditRequest::Transform { range, transform } => Self::Transform { range: range.into(), transform: transform.into() },
            EditRequest::FillEmpty { range, from_above, value } => Self::FillEmpty { range: range.into(), from_above, value },
            EditRequest::FixGarbled { range } => Self::FixGarbled { range: range.map(Into::into) },
            EditRequest::AddFormulaColumn { formula, name, at } => Self::AddFormulaColumn { formula, name, at: at as usize },
            EditRequest::FillWithFormula { formula, column } => Self::FillWithFormula { formula, column: column as usize },
            EditRequest::SplitColumn { column, separator, regex } => Self::SplitColumn { column: column as usize, separator, regex },
            EditRequest::MergeColumns { columns: list, separator } => Self::MergeColumns { columns: columns(list), separator },
            EditRequest::RemoveDuplicates { columns: list, normalize } => Self::RemoveDuplicates { columns: columns(list), normalize },
            EditRequest::ApplyRecipe { recipe } => Self::Recipe(recipe.get()),
        }
    }
}

/// What an edit did: cells changed, columns added or rows removed, and for formulas the report.
#[derive(uniffi::Record)]
pub struct EditResult {
    /// Whether the document changed; an undoable step was recorded.
    pub changed: bool,
    pub count: u64,
    pub report: Option<FormulaReport>,
}

impl From<core::EditOutcome> for EditResult {
    fn from(outcome: core::EditOutcome) -> Self {
        Self {
            changed: outcome.changed,
            count: outcome.count as u64,
            report: outcome.report.map(Into::into),
        }
    }
}

/// A change computed off the document; see `CoreDocument::begin_edit`.
#[derive(uniffi::Object)]
pub struct EditJob {
    /// The task, and the recipe step that repeats it while recording.
    task: Mutex<Option<(core::EditTask, Option<core::RecipeStep>)>>,
    progress: core::Progress,
}

#[uniffi::export]
impl EditJob {
    /// Computes the change; call off the main thread. Returns `false` when cancelled.
    pub fn run(&self) -> bool {
        let mut task = self.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        task.as_mut().is_some_and(|(task, _)| task.run(&self.progress))
    }

    pub fn cancel(&self) {
        self.progress.cancel();
    }

    /// Share of the work done, from 0 to 1.
    pub fn fraction(&self) -> f64 {
        self.progress.fraction()
    }
}

impl CoreDocument {
    fn wrap(document: core::Document) -> Arc<Self> {
        Self::wrap_indexing(document, None)
    }

    fn wrap_indexing(document: core::Document, indexer: Option<core::Indexer>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(document),
            indexer: Mutex::new(indexer),
            editing: AtomicBool::new(false),
            recorder: Mutex::new(None),
        })
    }

    fn lock(&self) -> MutexGuard<'_, core::Document> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A snapshot for work that only reads and takes a while: it runs without holding the
    /// document, so the table stays responsive meanwhile.
    fn read(&self) -> core::Document {
        self.lock().snapshot()
    }

    /// The document for a change, which must wait until indexing and a background edit end.
    fn edit(&self) -> Result<MutexGuard<'_, core::Document>, CoreError> {
        let document = self.whole()?;
        if self.editing.load(Ordering::Acquire) {
            return Err(CoreError::Editing);
        }
        Ok(document)
    }

    fn recorder(&self) -> MutexGuard<'_, Option<Recorder>> {
        self.recorder.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The step that repeats a change about to be made, while recording.
    fn step(&self, make: impl FnOnce() -> Option<core::RecipeStep>) -> Option<core::RecipeStep> {
        if self.recorder().is_some() { make() } else { None }
    }

    /// Records `step` if the change made after `before` was recorded as a new undo step.
    fn record(&self, document: &core::Document, before: Option<u64>, step: Option<core::RecipeStep>) {
        let (Some(step), Some(change)) = (step, document.last_change_id()) else { return };
        if Some(change) == before {
            return;
        }
        if let Some(recorder) = self.recorder().as_mut() {
            recorder.steps.push((step, change));
            recorder.undone.clear();
        }
    }

    /// The document for a save or an export, which must wait until indexing is done. A
    /// background edit does not change the document until it finishes, so it need not wait.
    fn whole(&self) -> Result<MutexGuard<'_, core::Document>, CoreError> {
        let document = self.lock();
        if document.is_indexing() {
            return Err(CoreError::Busy);
        }
        Ok(document)
    }
}

#[uniffi::export]
impl CoreDocument {
    #[uniffi::constructor]
    pub fn new_empty() -> Arc<Self> {
        Self::wrap(core::Document::new_empty())
    }

    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, CoreError> {
        Ok(Self::wrap(core::Document::open(path)?))
    }

    /// Opens a file with the given encoding and delimiter instead of detecting them.
    ///
    /// A large file opens with its first part indexed; index the rest with `continue_indexing`.
    #[uniffi::constructor]
    pub fn open_with(path: String, encoding: Option<TextEncoding>, delimiter: Option<String>) -> Result<Arc<Self>, CoreError> {
        let (document, indexer) = core::Document::open_partially(path, &open_options(encoding, delimiter)?, core::Document::FIRST_PART)?;
        Ok(Self::wrap_indexing(document, indexer))
    }

    /// Indexes about `bytes` more bytes of a large file; call off the main thread until done.
    pub fn continue_indexing(&self, bytes: u64) -> IndexProgress {
        let mut indexer = self.indexer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(active) = indexer.as_mut() else {
            return IndexProgress { done: true, indexed_bytes: 0, total_bytes: 0 };
        };
        // Scanning holds only the indexer; the document is locked just to add the rows.
        let chunk = active.next_chunk(bytes as usize);
        let done = chunk.is_done();
        let (indexed, total) = active.progress();
        self.lock().append_index(chunk);
        if done {
            *indexer = None;
        }
        IndexProgress { done, indexed_bytes: indexed as u64, total_bytes: total as u64 }
    }

    pub fn is_indexing(&self) -> bool {
        self.lock().is_indexing()
    }

    /// Prepares sorting and filtering to run with `ViewJob::run` off the main thread.
    pub fn begin_view(&self, spec: ViewSpec) -> Result<Arc<ViewJob>, CoreError> {
        let task = self.edit()?.begin_view(spec.into())?;
        Ok(Arc::new(ViewJob { task: Mutex::new(Some(task)), cancel: AtomicBool::new(false) }))
    }

    /// Shows the result of a job. `false` if it was cancelled or the rows changed meanwhile.
    pub fn finish_view(&self, job: Arc<ViewJob>) -> bool {
        let task = job.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        task.is_some_and(|task| self.lock().finish_view(task))
    }

    /// Prepares a change to many rows to run with `EditJob::run` off the main thread. Until
    /// `finish_edit`, other changes fail with `Editing`; reading, saving and exporting work.
    pub fn begin_edit(&self, request: EditRequest) -> Result<Arc<EditJob>, CoreError> {
        let document = self.edit()?;
        let edit: core::Edit = request.into();
        let step = self.step(|| document.recipe_step_for_edit(&edit));
        let task = document.begin_edit(edit);
        self.editing.store(true, Ordering::Release);
        Ok(Arc::new(EditJob { task: Mutex::new(Some((task, step))), progress: core::Progress::default() }))
    }

    /// Applies a job as one undoable step and lets other changes run again. `None` when it
    /// was cancelled; call it then too.
    pub fn finish_edit(&self, job: Arc<EditJob>) -> Result<Option<EditResult>, CoreError> {
        let task = job.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let mut document = self.lock();
        self.editing.store(false, Ordering::Release);
        let Some((task, step)) = task else { return Ok(None) };
        let before = document.last_change_id();
        let result = document.finish_edit(task)?;
        self.record(&document, before, step);
        Ok(result.map(Into::into))
    }

    /// Makes a change to many rows at once, for tables small enough not to need a job.
    pub fn apply_edit(&self, request: EditRequest) -> Result<EditResult, CoreError> {
        let edit: core::Edit = request.into();
        let mut document = self.edit()?;
        let step = self.step(|| document.recipe_step_for_edit(&edit));
        let before = document.last_change_id();
        let result = document.apply_edit(&edit)?;
        self.record(&document, before, step);
        Ok(result.into())
    }

    /// Whether an edit job runs.
    pub fn is_editing(&self) -> bool {
        self.editing.load(Ordering::Acquire)
    }

    #[uniffi::constructor]
    pub fn from_bytes(data: Vec<u8>) -> Arc<Self> {
        Self::wrap(core::Document::from_bytes(data))
    }

    pub fn info(&self) -> DocumentInfo {
        let document = self.lock();
        let info = document.info();
        DocumentInfo {
            encoding: info.encoding.name().to_owned(),
            has_bom: info.has_bom,
            delimiter: (info.dialect.delimiter as char).to_string(),
            line_ending: info.dialect.line_ending.into(),
            quote_style: match info.dialect.quote_style {
                core::QuoteStyle::Minimal => QuoteStyle::Minimal,
                core::QuoteStyle::Always => QuoteStyle::Always,
            },
            has_header: info.has_header,
            trailing_newline: info.trailing_newline,
            row_count: info.row_count as u64,
            column_count: info.column_count as u64,
            visible_row_count: info.visible_row_count as u64,
            unterminated_quote: info.unterminated_quote,
            text_encoding: info.encoding.into(),
            detection_confident: info.detection_confident,
            is_sheet: document.is_sheet(),
            sheet_rows_fixed: document.sheet_rows_fixed(),
            sheet_structure_locked: document.sheet_structure_locked(),
        }
    }

    pub fn row_count(&self) -> u64 {
        self.lock().row_count() as u64
    }

    pub fn column_count(&self) -> u64 {
        self.lock().column_count() as u64
    }

    pub fn row(&self, index: u64) -> Result<Vec<String>, CoreError> {
        Ok(self.lock().row(index as usize)?)
    }

    /// Up to `count` rows starting at `start`; fewer near the end of the document.
    pub fn rows(&self, start: u64, count: u64) -> Vec<Vec<String>> {
        let document = self.lock();
        let start = start as usize;
        let end = document.row_count().min(start.saturating_add(count as usize));
        (start..end)
            .map(|index| document.row(index).unwrap_or_default())
            .collect()
    }

    /// Returns `false` if the cell already held `value` and nothing was recorded.
    pub fn set_cell(&self, row: u64, column: u64, value: String) -> Result<bool, CoreError> {
        let (row, column) = (row as usize, column as usize);
        let mut document = self.edit()?;
        let step = self.step(|| document.recipe_step_for_set_cell(row, column, &value));
        let before = document.last_change_id();
        let changed = document.set_cell(row, column, &value)?;
        self.record(&document, before, step);
        Ok(changed)
    }

    pub fn insert_rows(&self, at: u64, count: u64) -> Result<(), CoreError> {
        Ok(self.edit()?.insert_rows(at as usize, count as usize)?)
    }

    pub fn delete_rows(&self, rows: Vec<u64>) -> Result<(), CoreError> {
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        let mut document = self.edit()?;
        let step = self.step(|| document.recipe_step_for_delete_rows(&rows));
        let before = document.last_change_id();
        document.delete_rows(&rows)?;
        self.record(&document, before, step);
        Ok(())
    }

    /// Moves rows to stand before row `to` (an index before the move), keeping their order.
    pub fn move_rows(&self, rows: Vec<u64>, to: u64) -> Result<bool, CoreError> {
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        Ok(self.edit()?.move_rows(&rows, to as usize)?)
    }

    /// Moves column `from` to index `to` of the resulting columns. Resets the view.
    pub fn move_column(&self, from: u64, to: u64) -> Result<bool, CoreError> {
        let (from, to) = (from as usize, to as usize);
        let mut document = self.edit()?;
        let step = self.step(|| Some(document.recipe_step_for_move_column(from, to)));
        let before = document.last_change_id();
        let moved = document.move_column(from, to)?;
        self.record(&document, before, step);
        Ok(moved)
    }

    pub fn insert_column(&self, at: u64) -> Result<(), CoreError> {
        let mut document = self.edit()?;
        let step = self.step(|| Some(document.recipe_step_for_insert_column(at as usize)));
        let before = document.last_change_id();
        document.insert_column(at as usize)?;
        self.record(&document, before, step);
        Ok(())
    }

    pub fn delete_column(&self, at: u64) -> Result<(), CoreError> {
        let mut document = self.edit()?;
        let step = self.step(|| Some(document.recipe_step_for_delete_column(at as usize)));
        let before = document.last_change_id();
        document.delete_column(at as usize)?;
        self.record(&document, before, step);
        Ok(())
    }

    // Visible rows: data rows as the current sort and filters show them.

    pub fn view_spec(&self) -> ViewSpec {
        self.lock().view_spec().into()
    }

    pub fn set_view(&self, spec: ViewSpec) -> Result<(), CoreError> {
        Ok(self.edit()?.set_view(spec.into())?)
    }

    /// Up to `count` visible rows starting at `start`.
    pub fn visible_rows(&self, start: u64, count: u64) -> Vec<Vec<String>> {
        let document = self.lock();
        let start = start as usize;
        let end = document
            .visible_row_count()
            .min(start.saturating_add(count as usize));
        (start..end)
            .map(|row| document.visible_row(row).unwrap_or_default())
            .collect()
    }

    /// The formula of a cell of a sheet, without its leading `=`.
    pub fn formula(&self, row: u64, column: u64) -> Option<String> {
        self.lock().formula(row as usize, column as usize)
    }

    /// The first cell, by row of the file and column, of the merged area that hides a cell.
    pub fn merged_into(&self, row: u64, column: u64) -> Option<Vec<u64>> {
        self.lock().merged_into(row as usize, column as usize).map(|(row, column)| vec![row as u64, column as u64])
    }

    /// What was typed into a cell of a sheet that shows it in its number format: `300` for a
    /// cell that shows `300,00 ₽`; `None` for cells that hold what they show.
    pub fn cell_input(&self, row: u64, column: u64) -> Option<String> {
        self.lock().cell_input(row as usize, column as usize)
    }

    /// Columns with a formula in each of up to `count` visible rows from `start`, as
    /// `visible_rows` returns them.
    pub fn visible_formula_columns(&self, start: u64, count: u64) -> Vec<Vec<u64>> {
        let document = self.lock();
        let start = start as usize;
        let end = document.visible_row_count().min(start.saturating_add(count as usize));
        (start..end)
            .map(|visible| {
                document
                    .visible_record(visible)
                    .map(|row| document.formula_columns(row).into_iter().map(|column| column as u64).collect())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Row of the file shown as visible row `visible_row`.
    pub fn visible_record(&self, visible_row: u64) -> Option<u64> {
        self.lock()
            .visible_record(visible_row as usize)
            .map(|row| row as u64)
    }

    /// Where row `row` of the file is shown, if it is visible.
    pub fn visible_index(&self, row: u64) -> Option<u64> {
        self.lock().visible_index(row as usize).map(|index| index as u64)
    }

    /// Row of the file where rows inserted before `visible_row` go.
    pub fn insertion_row(&self, visible_row: u64) -> u64 {
        self.lock().insertion_row(visible_row as usize) as u64
    }

    /// Reorders the file by the current sort; returns `false` if there was nothing to apply.
    pub fn apply_sort(&self) -> Result<bool, CoreError> {
        let mut document = self.edit()?;
        let step = self.step(|| document.recipe_step_for_apply_sort());
        let before = document.last_change_id();
        let sorted = document.apply_sort()?;
        self.record(&document, before, step);
        Ok(sorted)
    }

    pub fn find(
        &self,
        query: SearchQuery,
        from: Option<CellPosition>,
        forward: bool,
    ) -> Result<Option<CellPosition>, CoreError> {
        let from = from.map(|cell| (cell.row as usize, cell.column as usize));
        let found = self.read().find(&query.into(), from, forward)?;
        Ok(found.map(|(row, column)| CellPosition {
            row: row as u64,
            column: column as u64,
        }))
    }

    pub fn count_matches(&self, query: SearchQuery) -> Result<u64, CoreError> {
        Ok(self.read().count_matches(&query.into())? as u64)
    }

    /// Replaces the matches in one visible cell; `false` if it does not match.
    pub fn replace_in_cell(
        &self,
        visible_row: u64,
        column: u64,
        query: SearchQuery,
        replacement: String,
    ) -> Result<bool, CoreError> {
        Ok(self.edit()?.replace_in_cell(
            visible_row as usize,
            column as usize,
            &query.into(),
            &replacement,
        )?)
    }

    // Ranges of visible cells.

    pub fn copy_range(&self, range: CellRange) -> Result<String, CoreError> {
        Ok(self.read().copy_range(range.into())?)
    }

    pub fn range_stats(&self, range: CellRange) -> Result<RangeStats, CoreError> {
        let stats = self.lock().range_stats(range.into())?;
        Ok(RangeStats {
            cells: stats.cells as u64,
            filled: stats.filled as u64,
            numbers: stats.numbers as u64,
            sum: stats.sum,
        })
    }

    /// Sets every cell of the range; returns the number of changed cells.
    pub fn fill_range(&self, range: CellRange, value: String) -> Result<u64, CoreError> {
        Ok(self.edit()?.fill_range(range.into(), &value)? as u64)
    }

    pub fn fill_down(&self, range: CellRange) -> Result<u64, CoreError> {
        Ok(self.edit()?.fill_down(range.into())? as u64)
    }

    /// Writes `grid` from (`top`, `left`); returns the range written.
    pub fn paste(&self, top: u64, left: u64, grid: Vec<Vec<String>>) -> Result<Option<CellRange>, CoreError> {
        Ok(self
            .lock()
            .paste(top as usize, left as usize, &grid)?
            .map(Into::into))
    }

    /// Writes the visible rows to `path`, building them a batch at a time off the document.
    pub fn export_to_file(&self, path: String, format: ExportFormat, name: String) -> Result<(), CoreError> {
        let snapshot = self.whole()?.snapshot();
        Ok(snapshot.export_to_file(path, format.into(), &name)?)
    }

    /// Visible rows of `range` (all visible cells when `None`) in `format`. `name` titles the
    /// HTML page, SQL table and sheet; a `fragment` is for the clipboard.
    pub fn export(
        &self,
        format: ExportFormat,
        range: Option<CellRange>,
        name: String,
        fragment: bool,
    ) -> Result<Vec<u8>, CoreError> {
        let format: core::ExportFormat = format.into();
        let snapshot = self.whole()?.snapshot();
        Ok(snapshot.export(format, range.map(Into::into), &name, fragment)?)
    }

    pub fn profile_column(&self, column: u64) -> Result<ColumnProfile, CoreError> {
        let profile = self.read().profile_column(column as usize)?;
        Ok(ColumnProfile {
            kind: profile.kind.into(),
            total: profile.total as u64,
            empty: profile.empty as u64,
            unique: profile.unique as u64,
            mismatched: profile.mismatched as u64,
            padded: profile.padded as u64,
            min: profile.min,
            max: profile.max,
            mean: profile.mean,
            median: profile.median,
            min_length: profile.min_length as u64,
            max_length: profile.max_length as u64,
            top_values: profile
                .top_values
                .into_iter()
                .map(|(value, count)| ValueCount { value, count: count as u64 })
                .collect(),
            histogram: profile.histogram.into_iter().map(|count| count as u64).collect(),
        })
    }

    /// The next visible row after `after` whose value in `column` has `issue`, wrapping around.
    pub fn find_issue(&self, column: u64, issue: Issue, kind: ValueKind, after: Option<u64>) -> Option<u64> {
        let issue = match issue {
            Issue::Mismatch => core::Issue::Mismatch,
            Issue::Padding => core::Issue::Padding,
        };
        self.read()
            .find_issue(column as usize, issue, kind.into(), after.map(|row| row as usize))
            .map(|row| row as u64)
    }

    // Repair.

    /// Broken rows: the first `limit` of them, and counts of all.
    pub fn find_problems(&self, limit: u64) -> RepairReport {
        let report = self.read().find_problems(limit as usize);
        RepairReport {
            expected_fields: report.expected_fields as u64,
            problems: report
                .problems
                .into_iter()
                .map(|problem| Problem {
                    row: problem.row as u64,
                    kind: problem.kind.into(),
                    fields: problem.fields as u64,
                    lines: problem.lines as u64,
                })
                .collect(),
            counts: report
                .counts
                .into_iter()
                .map(|(kind, count)| ProblemCount { kind: kind.into(), count: count as u64 })
                .collect(),
            garbled_sample: report.garbled_sample as u64,
        }
    }

    /// Fixes for the problem in `row`, the recommended one first.
    pub fn repair_options(&self, row: u64) -> Result<Vec<RepairOption>, CoreError> {
        Ok(self
            .read()
            .repair_options(row as usize)?
            .into_iter()
            .map(|option| RepairOption {
                fix: option.fix.into(),
                rows: option.rows,
                row_count: option.row_count as u64,
            })
            .collect())
    }

    pub fn apply_fix(&self, row: u64, fix: RepairFix) -> Result<bool, CoreError> {
        Ok(self.edit()?.apply_fix(row as usize, fix.into())?)
    }

    /// Applies the recommended fix to every problem of one kind; returns the rows fixed.
    pub fn fix_all(&self, kind: ProblemKind) -> u64 {
        let Ok(mut document) = self.edit() else { return 0 };
        let kind = kind.into();
        let step = self.step(|| Some(document.recipe_step_for_fix_rows(kind)));
        let before = document.last_change_id();
        let fixed = document.fix_all(kind);
        self.record(&document, before, step);
        fixed as u64
    }

    /// The row as written in the file, cut to `max_chars` characters.
    pub fn raw_text(&self, row: u64, max_chars: u64) -> Result<String, CoreError> {
        Ok(self.lock().raw_text(row as usize, max_chars as usize)?)
    }

    // Formulas.

    /// Values of a formula for visible rows `start..start + count`.
    pub fn preview_formula(&self, formula: String, start: u64, count: u64) -> Result<Vec<FormulaOutcome>, CoreError> {
        let start = start as usize;
        Ok(self
            .read()
            .preview_formula(&formula, start..start + count as usize)?
            .into_iter()
            .map(|outcome| match outcome {
                Ok(value) => FormulaOutcome { value: Some(value), problem: None },
                Err(error) => FormulaOutcome { value: None, problem: Some(error.into()) },
            })
            .collect())
    }

    pub fn set_has_header(&self, has_header: bool) {
        if let Ok(mut document) = self.edit() {
            document.set_has_header(has_header);
        }
    }

    /// Whether the core has a step to undo: old steps of large changes are dropped to save memory.
    pub fn can_undo(&self) -> bool {
        self.lock().can_undo()
    }

    pub fn undo(&self) -> bool {
        let Ok(mut document) = self.edit() else { return false };
        let change = document.last_change_id();
        if !document.undo() {
            return false;
        }
        // Undoing a recorded change takes its step out of the recipe.
        if let Some(recorder) = self.recorder().as_mut()
            && recorder.steps.last().is_some_and(|(_, id)| Some(*id) == change)
        {
            recorder.undone.extend(recorder.steps.pop());
        }
        true
    }

    pub fn redo(&self) -> bool {
        let Ok(mut document) = self.edit() else { return false };
        if !document.redo() {
            return false;
        }
        let change = document.last_change_id();
        if let Some(recorder) = self.recorder().as_mut()
            && recorder.undone.last().is_some_and(|(_, id)| Some(*id) == change)
        {
            recorder.steps.extend(recorder.undone.pop());
        }
        true
    }

    /// The format the file had when it was opened.
    pub fn source_format(&self) -> SaveFormat {
        self.lock().source_format().into()
    }

    /// The format saves use; changed by "Save As".
    pub fn save_format(&self) -> SaveFormat {
        self.lock().save_format().into()
    }

    pub fn set_save_format(&self, format: SaveFormat) -> Result<(), CoreError> {
        Ok(self.lock().set_save_format(format.try_into()?)?)
    }

    /// Writes a copy in `format`; the document keeps its own format (for "Export").
    pub fn save_with(&self, path: String, format: SaveFormat) -> Result<(), CoreError> {
        Ok(self.whole()?.save_with(path, &format.try_into()?)?)
    }

    pub fn save(&self, path: String) -> Result<(), CoreError> {
        Ok(self.whole()?.save(path)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.whole()?.to_bytes()?)
    }
}

// Comparing two tables.

#[derive(uniffi::Record)]
pub struct DiffOptions {
    /// Key columns of this table; empty compares rows by position.
    pub key: Vec<u64>,
    pub ignore_case: bool,
    pub trim: bool,
}

#[derive(uniffi::Record)]
pub struct DiffColumnInfo {
    pub name: Option<String>,
    pub left: Option<u64>,
    pub right: Option<u64>,
}

impl From<&core::DiffColumn> for DiffColumnInfo {
    fn from(column: &core::DiffColumn) -> Self {
        Self {
            name: column.name.clone(),
            left: column.left.map(|c| c as u64),
            right: column.right.map(|c| c as u64),
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffStatus {
    OnlyLeft,
    OnlyRight,
    Changed,
    Same,
}

impl From<core::DiffStatus> for DiffStatus {
    fn from(status: core::DiffStatus) -> Self {
        match status {
            core::DiffStatus::OnlyLeft => Self::OnlyLeft,
            core::DiffStatus::OnlyRight => Self::OnlyRight,
            core::DiffStatus::Changed => Self::Changed,
            core::DiffStatus::Same => Self::Same,
        }
    }
}

impl From<DiffStatus> for core::DiffStatus {
    fn from(status: DiffStatus) -> Self {
        match status {
            DiffStatus::OnlyLeft => Self::OnlyLeft,
            DiffStatus::OnlyRight => Self::OnlyRight,
            DiffStatus::Changed => Self::Changed,
            DiffStatus::Same => Self::Same,
        }
    }
}

#[derive(uniffi::Record)]
pub struct DiffSummary {
    pub only_left: u64,
    pub only_right: u64,
    pub changed: u64,
    pub same: u64,
    pub repeated_left: u64,
    pub repeated_right: u64,
}

#[derive(uniffi::Record)]
pub struct DiffCellValue {
    pub left: Option<String>,
    pub right: Option<String>,
    pub changed: bool,
}

/// A row of the comparison; rows are rows of the files, header included.
#[derive(uniffi::Record)]
pub struct DiffRow {
    pub status: DiffStatus,
    pub left_row: Option<u64>,
    pub right_row: Option<u64>,
    pub cells: Vec<DiffCellValue>,
}

/// A comparison computed off the main thread; see `CoreDocument::begin_diff`.
#[derive(uniffi::Object)]
pub struct DiffJob {
    tables: Mutex<Option<(core::Document, core::Document)>>,
    options: core::DiffOptions,
    progress: core::Progress,
}

#[uniffi::export]
impl DiffJob {
    /// Compares the tables; call off the main thread. `None` when cancelled.
    pub fn run(&self) -> Result<Option<Arc<DiffResult>>, CoreError> {
        let tables = self.tables.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let Some((left, right)) = tables else { return Ok(None) };
        let diff = left.diff(&right, &self.options, &self.progress)?;
        Ok(diff.map(|diff| Arc::new(DiffResult { diff, shown: Mutex::new(None) })))
    }

    pub fn cancel(&self) {
        self.progress.cancel();
    }

    pub fn fraction(&self) -> f64 {
        self.progress.fraction()
    }
}

/// Entries of a comparison with some statuses, by their index.
type ShownEntries = (Vec<DiffStatus>, Arc<Vec<u32>>);

/// The result of a comparison: rows are read a few at a time, for the statuses shown.
#[derive(uniffi::Object)]
pub struct DiffResult {
    diff: core::Diff,
    /// Entries with the statuses last asked for.
    shown: Mutex<Option<ShownEntries>>,
}

impl DiffResult {
    fn shown(&self, statuses: &[DiffStatus]) -> Arc<Vec<u32>> {
        let mut shown = self.shown.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached, entries)) = shown.as_ref()
            && cached == statuses
        {
            return Arc::clone(entries);
        }
        let entries: Arc<Vec<u32>> = Arc::new(
            self.diff
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| statuses.contains(&entry.status.into()))
                .map(|(index, _)| index as u32)
                .collect(),
        );
        *shown = Some((statuses.to_vec(), Arc::clone(&entries)));
        entries
    }
}

#[uniffi::export]
impl DiffResult {
    pub fn summary(&self) -> DiffSummary {
        let counts = self.diff.counts();
        DiffSummary {
            only_left: counts.only_left as u64,
            only_right: counts.only_right as u64,
            changed: counts.changed as u64,
            same: counts.same as u64,
            repeated_left: counts.repeated_left as u64,
            repeated_right: counts.repeated_right as u64,
        }
    }

    pub fn columns(&self) -> Vec<DiffColumnInfo> {
        self.diff.columns().iter().map(Into::into).collect()
    }

    /// Number of rows with one of `statuses`.
    pub fn count(&self, statuses: Vec<DiffStatus>) -> u64 {
        self.shown(&statuses).len() as u64
    }

    /// Rows `start..start + count` of those with one of `statuses`.
    pub fn rows(&self, statuses: Vec<DiffStatus>, start: u64, count: u64) -> Vec<DiffRow> {
        let shown = self.shown(&statuses);
        let start = (start as usize).min(shown.len());
        let end = (start + count as usize).min(shown.len());
        shown[start..end]
            .iter()
            .map(|&index| {
                let entry = &self.diff.entries()[index as usize];
                DiffRow {
                    status: entry.status.into(),
                    left_row: entry.left.map(|row| row as u64),
                    right_row: entry.right.map(|row| row as u64),
                    cells: self
                        .diff
                        .cells(entry)
                        .into_iter()
                        .map(|cell| DiffCellValue { left: cell.left, right: cell.right, changed: cell.changed })
                        .collect(),
                }
            })
            .collect()
    }

    /// Rows with one of `statuses` as a CSV table with a status column titled `status_title`;
    /// `names` name the statuses in the order only left, only right, changed, same.
    pub fn to_csv(&self, statuses: Vec<DiffStatus>, names: Vec<String>, status_title: String) -> Vec<u8> {
        let statuses: Vec<core::DiffStatus> = statuses.into_iter().map(Into::into).collect();
        let name = |index: usize| names.get(index).map(String::as_str).unwrap_or_default();
        self.diff.to_csv(&statuses, [name(0), name(1), name(2), name(3)], &status_title)
    }
}

#[uniffi::export]
impl CoreDocument {
    /// How the columns of this table and `other` line up.
    pub fn diff_columns(&self, other: Arc<CoreDocument>) -> Vec<DiffColumnInfo> {
        let other = other.read();
        self.read().diff_columns(&other).iter().map(Into::into).collect()
    }

    /// Key columns of this table that suit a comparison with `other`; empty if none does.
    pub fn suggest_diff_key(&self, other: Arc<CoreDocument>) -> Vec<u64> {
        let other = other.read();
        self.read().suggest_diff_key(&other).into_iter().map(|c| c as u64).collect()
    }

    /// Prepares a comparison of this table (left) with `other` (right) to run with
    /// `DiffJob::run` off the main thread. Both must be read whole.
    pub fn begin_diff(&self, other: Arc<CoreDocument>, options: DiffOptions) -> Result<Arc<DiffJob>, CoreError> {
        let left = self.whole()?.snapshot();
        let right = other.whole()?.snapshot();
        Ok(Arc::new(DiffJob {
            tables: Mutex::new(Some((left, right))),
            options: core::DiffOptions {
                key: options.key.into_iter().map(|c| c as usize).collect(),
                ignore_case: options.ignore_case,
                trim: options.trim,
            },
            progress: core::Progress::default(),
        }))
    }
}

// Recipes.

/// A recipe: steps that repeat changes on another file, found by column names.
#[derive(uniffi::Object)]
pub struct CoreRecipe {
    inner: Mutex<core::Recipe>,
}

/// One step of a recipe for a list: a kind (`replace`, `uppercase`…) and its arguments.
#[derive(uniffi::Record)]
pub struct RecipeStepInfo {
    pub kind: String,
    pub arguments: Vec<String>,
    /// The filter that selected the rows the step works on when it was recorded.
    pub filters: Vec<RecipeFilterInfo>,
}

/// A filter condition: `op` is `contains`, `equals`, `greater_than`…; any column when `None`.
#[derive(uniffi::Record)]
pub struct RecipeFilterInfo {
    pub column: Option<String>,
    pub op: String,
    pub value: String,
}

impl CoreRecipe {
    fn get(&self) -> core::Recipe {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn with<T>(&self, change: impl FnOnce(&mut core::Recipe) -> T) -> T {
        change(&mut self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }
}

#[uniffi::export]
impl CoreRecipe {
    #[uniffi::constructor]
    pub fn from_json(json: String) -> Result<Arc<Self>, CoreError> {
        Ok(Arc::new(Self { inner: Mutex::new(core::Recipe::from_json(&json)?) }))
    }

    pub fn to_json(&self) -> String {
        self.get().to_json()
    }

    pub fn name(&self) -> String {
        self.get().name
    }

    pub fn set_name(&self, name: String) {
        self.with(|recipe| recipe.name = name);
    }

    pub fn steps(&self) -> Vec<RecipeStepInfo> {
        self.get()
            .steps
            .iter()
            .map(|step| {
                let (kind, arguments) = step.describe();
                let filters = step
                    .filter_conditions()
                    .into_iter()
                    .map(|(column, op, value)| RecipeFilterInfo { column, op: op.to_owned(), value })
                    .collect();
                RecipeStepInfo { kind: kind.to_owned(), arguments, filters }
            })
            .collect()
    }

    pub fn remove_step(&self, index: u64) {
        self.with(|recipe| {
            if (index as usize) < recipe.steps.len() {
                recipe.steps.remove(index as usize);
            }
        });
    }

    /// The recipe as a Python script with pandas reading `input` and writing `output`.
    pub fn to_pandas(&self, input: String, output: String, delimiter: String) -> String {
        self.get().to_pandas(&input, &output, delimiter.chars().next().unwrap_or(','))
    }
}

#[uniffi::export]
impl CoreDocument {
    /// Starts recording changes as recipe steps; an earlier recording is dropped.
    pub fn start_recording(&self) {
        *self.recorder() = Some(Recorder::default());
    }

    /// Ends recording and returns the steps recorded.
    pub fn stop_recording(&self) -> Option<Arc<CoreRecipe>> {
        let recorder = self.recorder().take()?;
        let steps = recorder.steps.into_iter().map(|(step, _)| step).collect();
        Some(Arc::new(CoreRecipe { inner: Mutex::new(core::Recipe { name: String::new(), steps }) }))
    }

    /// Steps recorded so far; `None` when not recording.
    pub fn recorded_step_count(&self) -> Option<u64> {
        self.recorder().as_ref().map(|recorder| recorder.steps.len() as u64)
    }
}

#[cfg(test)]
mod recording_tests {
    use super::*;

    fn column(document: &CoreDocument, column: u64) -> CellRange {
        let rows = document.lock().visible_row_count() as u64;
        CellRange { top: 0, left: column, bottom: rows - 1, right: column }
    }

    #[test]
    fn undo_and_redo_take_steps_out_and_back() {
        let document = CoreDocument::from_bytes(b"a,b\nx,y\nz,w\n".to_vec());
        document.start_recording();
        document.apply_edit(EditRequest::Transform { range: column(&document, 0), transform: TextTransform::Uppercase }).unwrap();
        // A single cell does not repeat on another file.
        document.set_cell(1, 1, "q".into()).unwrap();
        document.delete_column(1).unwrap();
        assert_eq!(document.recorded_step_count(), Some(2));
        assert!(document.undo());
        assert_eq!(document.recorded_step_count(), Some(1));
        // Undoing the cell edit, which was not recorded, keeps the steps.
        assert!(document.undo());
        assert_eq!(document.recorded_step_count(), Some(1));
        assert!(document.redo());
        assert!(document.redo());
        assert_eq!(document.recorded_step_count(), Some(2));
        let recipe = document.stop_recording().unwrap();
        let kinds: Vec<String> = recipe.steps().into_iter().map(|step| step.kind).collect();
        assert_eq!(kinds, ["uppercase", "delete_column"]);
        assert_eq!(document.recorded_step_count(), None);
    }
}

// MARK: Workbooks

#[derive(uniffi::Enum)]
pub enum SheetVisibility {
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(uniffi::Record)]
pub struct SheetInfo {
    pub name: String,
    pub visibility: SheetVisibility,
    /// A sheet of cells; charts and old macro sheets are not.
    pub is_table: bool,
}

/// How the system shows numbers and dates, which Excel follows.
#[derive(uniffi::Record)]
pub struct DisplayLocale {
    pub decimal_separator: String,
    pub group_separator: String,
    /// The built-in short date as an Excel format code, such as `dd.mm.yyyy`.
    pub short_date: String,
    /// January first.
    pub month_names: Vec<String>,
    pub month_abbreviations: Vec<String>,
    /// Sunday first.
    pub day_names: Vec<String>,
    pub day_abbreviations: Vec<String>,
    pub am: String,
    pub pm: String,
    pub true_text: String,
    pub false_text: String,
}

impl From<DisplayLocale> for core::DisplayLocale {
    fn from(locale: DisplayLocale) -> Self {
        let defaults = core::DisplayLocale::default();
        let names = |names: Vec<String>, count: usize, default: Vec<String>| if names.len() == count { names } else { default };
        let character = |text: &str, default: char| text.chars().next().unwrap_or(default);
        Self {
            decimal_separator: character(&locale.decimal_separator, defaults.decimal_separator),
            group_separator: character(&locale.group_separator, defaults.group_separator),
            short_date: if locale.short_date.is_empty() { defaults.short_date } else { locale.short_date },
            month_names: names(locale.month_names, 12, defaults.month_names),
            month_abbreviations: names(locale.month_abbreviations, 12, defaults.month_abbreviations),
            day_names: names(locale.day_names, 7, defaults.day_names),
            day_abbreviations: names(locale.day_abbreviations, 7, defaults.day_abbreviations),
            am: locale.am,
            pm: locale.pm,
            true_text: locale.true_text,
            false_text: locale.false_text,
        }
    }
}

/// Rows `top..=bottom` and columns `left..=right` of a sheet.
#[derive(uniffi::Record)]
pub struct SheetRange {
    pub top: u64,
    pub left: u64,
    pub bottom: u64,
    pub right: u64,
}

/// Columns `first..=last` as the sheet sets them.
#[derive(uniffi::Record)]
pub struct ColumnLayout {
    pub first: u64,
    pub last: u64,
    /// Width in characters of the workbook's default font.
    pub width: Option<f64>,
    pub hidden: bool,
}

#[derive(uniffi::Record)]
pub struct SheetLayout {
    pub merged: Vec<SheetRange>,
    pub columns: Vec<ColumnLayout>,
    pub default_column_width: Option<f64>,
    pub frozen_rows: u64,
    pub frozen_columns: u64,
}

impl From<core::SheetLayout> for SheetLayout {
    fn from(layout: core::SheetLayout) -> Self {
        Self {
            merged: layout
                .merged
                .into_iter()
                .map(|range| SheetRange {
                    top: range.top as u64,
                    left: range.left as u64,
                    bottom: range.bottom as u64,
                    right: range.right as u64,
                })
                .collect(),
            columns: layout
                .columns
                .into_iter()
                .map(|column| ColumnLayout {
                    first: column.first as u64,
                    last: column.last as u64,
                    width: column.width,
                    hidden: column.hidden,
                })
                .collect(),
            default_column_width: layout.default_column_width,
            frozen_rows: layout.frozen_rows as u64,
            frozen_columns: layout.frozen_columns as u64,
        }
    }
}

/// A sheet read from a workbook: its table, and how Excel lays it out.
#[derive(uniffi::Record)]
pub struct LoadedSheet {
    pub document: Arc<CoreDocument>,
    pub layout: SheetLayout,
}

/// Reading a sheet, with its progress and a way to cancel it; see `CoreWorkbook::read_sheet`.
#[derive(uniffi::Object)]
pub struct SheetJob {
    progress: core::Progress,
}

#[uniffi::export]
impl SheetJob {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { progress: core::Progress::default() })
    }

    pub fn cancel(&self) {
        self.progress.cancel();
    }

    /// Share of the sheet read, from 0 to 1.
    pub fn fraction(&self) -> f64 {
        self.progress.fraction()
    }
}

/// A sheet to save: its index in the workbook and the document read from it.
#[derive(uniffi::Record)]
pub struct SheetToSave {
    pub index: u64,
    pub document: Arc<CoreDocument>,
}

/// What computing formulas again did.
#[derive(uniffi::Record)]
pub struct RecalcReport {
    /// Sheets whose shown values changed.
    pub sheets: Vec<u64>,
    /// Formulas that read themselves through other cells; they show 0.
    pub circular: u64,
    /// Formulas with a function CommaEditor does not compute; Excel computes them on opening.
    pub not_computed: u64,
}

#[derive(uniffi::Enum)]
pub enum SheetAxis {
    Rows,
    Columns,
}

impl From<SheetAxis> for core::Axis {
    fn from(axis: SheetAxis) -> Self {
        match axis {
            SheetAxis::Rows => Self::Rows,
            SheetAxis::Columns => Self::Columns,
        }
    }
}

/// An open Excel workbook. Sheets are read one at a time.
#[derive(uniffi::Object)]
pub struct CoreWorkbook {
    inner: core::Workbook,
}

#[uniffi::export]
impl CoreWorkbook {
    #[uniffi::constructor]
    pub fn open(path: String) -> Result<Arc<Self>, CoreError> {
        Ok(Arc::new(Self { inner: core::Workbook::open(path)? }))
    }

    /// Sheets in the order of their tabs.
    pub fn sheets(&self) -> Vec<SheetInfo> {
        self.inner
            .sheets()
            .iter()
            .map(|sheet| SheetInfo {
                name: sheet.name.clone(),
                visibility: match sheet.visibility {
                    core::SheetVisibility::Visible => SheetVisibility::Visible,
                    core::SheetVisibility::Hidden => SheetVisibility::Hidden,
                    core::SheetVisibility::VeryHidden => SheetVisibility::VeryHidden,
                },
                is_table: sheet.kind == core::SheetKind::Worksheet,
            })
            .collect()
    }

    /// Whether the workbook is in the Excel 97–2003 format, which is only read.
    pub fn is_legacy_format(&self) -> bool {
        self.inner.format() == core::WorkbookFormat::Xls
    }

    /// The sheet to show first.
    pub fn active_sheet(&self) -> u64 {
        self.inner.active_sheet() as u64
    }

    /// Sheets to read and compute as soon as the workbook opens: the ones with formulas, in a
    /// file that asks to compute every formula on opening; empty for other files.
    pub fn sheets_computed_on_open(&self) -> Result<Vec<u64>, CoreError> {
        Ok(self.inner.sheets_computed_on_open()?.into_iter().map(|index| index as u64).collect())
    }

    /// The workbook's file with the edits of the given sheets. The file opened stays the base:
    /// unchanged parts, rows and cells are copied from it. The app writes the bytes where
    /// NSDocument asks, which in the sandbox is one temporary file it may write.
    pub fn bytes(&self, sheets: Vec<SheetToSave>) -> Result<Vec<u8>, CoreError> {
        let documents = sheets.iter().map(|sheet| sheet.document.whole()).collect::<Result<Vec<_>, _>>()?;
        let edited: Vec<(usize, &core::Document)> =
            sheets.iter().zip(&documents).map(|(sheet, document)| (sheet.index as usize, &**document)).collect();
        Ok(self.inner.to_bytes(&edited)?)
    }

    /// Computes again the formulas of the given sheets that depend on edited cells, and the
    /// ones entered. `now` is the date and time as an Excel serial number, for TODAY and NOW.
    pub fn recalculate(&self, sheets: Vec<SheetToSave>, now: f64) -> Result<RecalcReport, CoreError> {
        let mut documents = sheets.iter().map(|sheet| sheet.document.whole()).collect::<Result<Vec<_>, _>>()?;
        let mut edited: Vec<(usize, &mut core::Document)> =
            sheets.iter().zip(documents.iter_mut()).map(|(sheet, document)| (sheet.index as usize, &mut **document)).collect();
        let report = self.inner.recalculate(&mut edited, now)?;
        Ok(RecalcReport {
            sheets: report.sheets.into_iter().map(|sheet| sheet as u64).collect(),
            circular: report.circular as u64,
            not_computed: report.not_computed as u64,
        })
    }

    /// Inserts `count` rows or columns before `at` in sheet `index`, moving references in the
    /// given sheets, which must be every sheet with formulas. Returns the sheets changed; each
    /// took one undoable step.
    pub fn insert(&self, sheets: Vec<SheetToSave>, index: u64, axis: SheetAxis, at: u64, count: u64) -> Result<Vec<u64>, CoreError> {
        self.restructure(sheets, index, axis, at, count, true)
    }

    /// Deletes `count` rows or columns from `at` in sheet `index`; see `insert`.
    pub fn delete(&self, sheets: Vec<SheetToSave>, index: u64, axis: SheetAxis, at: u64, count: u64) -> Result<Vec<u64>, CoreError> {
        self.restructure(sheets, index, axis, at, count, false)
    }

    /// Moves rows (records) of sheet `index` to stand before row `to`, references following
    /// in every sheet given, as cutting and inserting rows does in Excel; returns the sheets
    /// changed, each with one undoable step.
    pub fn move_rows(&self, sheets: Vec<SheetToSave>, index: u64, rows: Vec<u64>, to: u64) -> Result<Vec<u64>, CoreError> {
        let mut documents = sheets.iter().map(|sheet| sheet.document.edit()).collect::<Result<Vec<_>, _>>()?;
        let mut edited: Vec<(usize, &mut core::Document)> =
            sheets.iter().zip(documents.iter_mut()).map(|(sheet, document)| (sheet.index as usize, &mut **document)).collect();
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        let changed = self.inner.move_rows(&mut edited, index as usize, &rows, to as usize)?;
        Ok(changed.into_iter().map(|sheet| sheet as u64).collect())
    }

    /// Splits a column of sheet `index` into new columns after it, references to the columns
    /// after it moving in every sheet given; each sheet changed took one undoable step.
    pub fn split_column(&self, sheets: Vec<SheetToSave>, index: u64, column: u64, separator: String, regex: bool) -> Result<ColumnSplit, CoreError> {
        let mut documents = sheets.iter().map(|sheet| sheet.document.edit()).collect::<Result<Vec<_>, _>>()?;
        let mut edited: Vec<(usize, &mut core::Document)> =
            sheets.iter().zip(documents.iter_mut()).map(|(sheet, document)| (sheet.index as usize, &mut **document)).collect();
        let (changed, count) = self.inner.split_column(&mut edited, index as usize, column as usize, &separator, regex)?;
        Ok(ColumnSplit { sheets: changed.into_iter().map(|sheet| sheet as u64).collect(), count: count as u64 })
    }

    /// Joins columns of sheet `index` into a new column after the last of them, references to
    /// the columns after it moving in every sheet given; returns the sheets changed.
    pub fn merge_columns(&self, sheets: Vec<SheetToSave>, index: u64, columns: Vec<u64>, separator: String) -> Result<Vec<u64>, CoreError> {
        let mut documents = sheets.iter().map(|sheet| sheet.document.edit()).collect::<Result<Vec<_>, _>>()?;
        let mut edited: Vec<(usize, &mut core::Document)> =
            sheets.iter().zip(documents.iter_mut()).map(|(sheet, document)| (sheet.index as usize, &mut **document)).collect();
        let columns: Vec<usize> = columns.into_iter().map(|column| column as usize).collect();
        let changed = self.inner.merge_columns(&mut edited, index as usize, &columns, &separator)?;
        Ok(changed.into_iter().map(|sheet| sheet as u64).collect())
    }

    /// Puts the rows of sheet `index` in the order its view sorts them, as Excel sorts:
    /// formulas and formats move with their rows, references from elsewhere stay. `false`
    /// when nothing moved.
    pub fn sort_rows(&self, document: Arc<CoreDocument>, index: u64) -> Result<bool, CoreError> {
        let mut edited = document.edit()?;
        Ok(self.inner.sort_rows(&mut edited, index as usize)?)
    }

    /// Reads a sheet; call off the main thread for large ones. `None` if `job` was cancelled.
    pub fn read_sheet(&self, index: u64, locale: DisplayLocale, job: Arc<SheetJob>) -> Result<Option<LoadedSheet>, CoreError> {
        let sheet = self.inner.read_sheet_with_progress(index as usize, &locale.into(), &job.progress)?;
        Ok(sheet.map(|sheet| LoadedSheet { document: CoreDocument::wrap(sheet.document), layout: sheet.layout.into() }))
    }
}

impl CoreWorkbook {
    fn restructure(&self, sheets: Vec<SheetToSave>, index: u64, axis: SheetAxis, at: u64, count: u64, insert: bool) -> Result<Vec<u64>, CoreError> {
        let mut documents = sheets.iter().map(|sheet| sheet.document.edit()).collect::<Result<Vec<_>, _>>()?;
        let mut edited: Vec<(usize, &mut core::Document)> =
            sheets.iter().zip(documents.iter_mut()).map(|(sheet, document)| (sheet.index as usize, &mut **document)).collect();
        let (index, at, count, axis) = (index as usize, at as usize, count as usize, axis.into());
        let changed = if insert {
            self.inner.insert(&mut edited, index, axis, at, count)?
        } else {
            self.inner.delete(&mut edited, index, axis, at, count)?
        };
        Ok(changed.into_iter().map(|sheet| sheet as u64).collect())
    }
}

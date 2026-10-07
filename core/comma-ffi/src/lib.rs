//! UniFFI bindings of `comma-core` for the macOS app.
//!
//! Indices cross the boundary as `u64`; rows include the header row, if there is one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use comma_core as core;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("{message}")]
    Io { message: String },
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
    #[error("{message}")]
    InvalidDelimiter { message: String },
    #[error("{message}")]
    Export { message: String },
    #[error("{problem:?}")]
    Formula { problem: FormulaProblem },
    /// The file is still being indexed; it cannot be changed or saved yet.
    #[error("the file is still being read")]
    Busy,
}

impl From<core::Error> for CoreError {
    fn from(error: core::Error) -> Self {
        match error {
            core::Error::Io(error) => Self::Io {
                message: error.to_string(),
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
            core::Error::InvalidDelimiter(_) => Self::InvalidDelimiter {
                message: error.to_string(),
            },
            core::Error::Export(message) => Self::Export { message },
            core::Error::Formula(error) => Self::Formula { problem: error.into() },
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
        }
    }
}

/// One ASCII character as a delimiter byte.
fn delimiter_byte(delimiter: &str) -> Result<u8, CoreError> {
    let mut chars = delimiter.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() && c != '"' && c != '\n' && c != '\r' => Ok(c as u8),
        _ => Err(CoreError::InvalidDelimiter {
            message: format!("“{delimiter}” cannot be used as a delimiter"),
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
                    message: format!("“{}” cannot be used as a delimiter", format.delimiter),
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

/// The first part of a file indexed when it opens; the rest follows with `continue_indexing`.
const FIRST_PART: usize = 32 << 20;

impl CoreDocument {
    fn wrap(document: core::Document) -> Arc<Self> {
        Self::wrap_indexing(document, None)
    }

    fn wrap_indexing(document: core::Document, indexer: Option<core::Indexer>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(document),
            indexer: Mutex::new(indexer),
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

    /// The document for a change or a save, which must wait until indexing is done.
    fn edit(&self) -> Result<MutexGuard<'_, core::Document>, CoreError> {
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
        let (document, indexer) = core::Document::open_partially(path, &open_options(encoding, delimiter)?, FIRST_PART)?;
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

    #[uniffi::constructor]
    pub fn from_bytes(data: Vec<u8>) -> Arc<Self> {
        Self::wrap(core::Document::from_bytes(data))
    }

    pub fn info(&self) -> DocumentInfo {
        let info = self.lock().info();
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
        Ok(self.edit()?.set_cell(row as usize, column as usize, &value)?)
    }

    pub fn insert_rows(&self, at: u64, count: u64) -> Result<(), CoreError> {
        Ok(self.edit()?.insert_rows(at as usize, count as usize)?)
    }

    pub fn delete_rows(&self, rows: Vec<u64>) -> Result<(), CoreError> {
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        Ok(self.edit()?.delete_rows(&rows)?)
    }

    /// Moves rows to stand before row `to` (an index before the move), keeping their order.
    pub fn move_rows(&self, rows: Vec<u64>, to: u64) -> Result<bool, CoreError> {
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        Ok(self.edit()?.move_rows(&rows, to as usize)?)
    }

    /// Moves column `from` to index `to` of the resulting columns. Resets the view.
    pub fn move_column(&self, from: u64, to: u64) -> Result<bool, CoreError> {
        Ok(self.edit()?.move_column(from as usize, to as usize)?)
    }

    pub fn insert_column(&self, at: u64) -> Result<(), CoreError> {
        Ok(self.edit()?.insert_column(at as usize)?)
    }

    pub fn delete_column(&self, at: u64) -> Result<(), CoreError> {
        Ok(self.edit()?.delete_column(at as usize)?)
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
        Ok(self.edit()?.apply_sort()?)
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

    /// Replaces every match in the visible rows; returns the number of changed cells.
    pub fn replace_all(&self, query: SearchQuery, replacement: String) -> Result<u64, CoreError> {
        Ok(self.edit()?.replace_all(&query.into(), &replacement)? as u64)
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

    /// Visible rows of `range` (all visible cells when `None`) in `format`. `name` titles the
    /// HTML page, SQL table and sheet; a `fragment` is for the clipboard.
    /// Writes the visible rows to `path`, building them a batch at a time off the document.
    pub fn export_to_file(&self, path: String, format: ExportFormat, name: String) -> Result<(), CoreError> {
        let snapshot = self.edit()?.snapshot();
        Ok(snapshot.export_to_file(path, format.into(), &name)?)
    }

    pub fn export(
        &self,
        format: ExportFormat,
        range: Option<CellRange>,
        name: String,
        fragment: bool,
    ) -> Result<Vec<u8>, CoreError> {
        let format: core::ExportFormat = format.into();
        let snapshot = self.edit()?.snapshot();
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
        self.edit().map_or(0, |mut document| document.fix_all(kind.into()) as u64)
    }

    /// Fixes text read in the wrong encoding in a range, or in the whole file with the header.
    pub fn fix_garbled_text(&self, range: Option<CellRange>) -> Result<u64, CoreError> {
        Ok(self.edit()?.fix_garbled_text(range.map(Into::into))? as u64)
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

    /// Inserts a column computed by the formula before column `at`.
    pub fn add_formula_column(&self, formula: String, name: String, at: u64) -> Result<FormulaReport, CoreError> {
        Ok(self.edit()?.add_formula_column(&formula, &name, at as usize)?.into())
    }

    /// Writes the formula's value into a column of every visible row.
    pub fn fill_column_with_formula(&self, formula: String, column: u64) -> Result<FormulaReport, CoreError> {
        Ok(self.edit()?.fill_column_with_formula(&formula, column as usize)?.into())
    }

    // Column operations.

    /// Transforms the text of every cell in the range; returns the number of changed cells.
    pub fn transform_range(&self, range: CellRange, transform: TextTransform) -> Result<u64, CoreError> {
        let transform = match transform {
            TextTransform::Trim => core::TextTransform::Trim,
            TextTransform::CollapseSpaces => core::TextTransform::CollapseSpaces,
            TextTransform::Uppercase => core::TextTransform::Uppercase,
            TextTransform::Lowercase => core::TextTransform::Lowercase,
            TextTransform::Capitalize => core::TextTransform::Capitalize,
        };
        Ok(self.edit()?.transform_range(range.into(), transform)? as u64)
    }

    /// Fills empty cells with the value above them or with `value`.
    pub fn fill_empty(&self, range: CellRange, from_above: bool, value: String) -> Result<u64, CoreError> {
        Ok(self.edit()?.fill_empty(range.into(), from_above, &value)? as u64)
    }

    /// Splits a column into new columns after it; returns how many were added.
    pub fn split_column(&self, column: u64, separator: String, regex: bool) -> Result<u64, CoreError> {
        Ok(self.edit()?.split_column(column as usize, &separator, regex)? as u64)
    }

    /// Joins columns into a new column after the last of them.
    pub fn merge_columns(&self, columns: Vec<u64>, separator: String) -> Result<(), CoreError> {
        let columns: Vec<usize> = columns.into_iter().map(|column| column as usize).collect();
        Ok(self.edit()?.merge_columns(&columns, &separator)?)
    }

    /// Deletes repeated visible rows; returns how many were removed.
    pub fn remove_duplicates(&self, columns: Vec<u64>, normalize: bool) -> Result<u64, CoreError> {
        let columns: Vec<usize> = columns.into_iter().map(|column| column as usize).collect();
        Ok(self.edit()?.remove_duplicates(&columns, normalize)? as u64)
    }

    pub fn set_has_header(&self, has_header: bool) {
        if let Ok(mut document) = self.edit() {
            document.set_has_header(has_header);
        }
    }

    pub fn undo(&self) -> bool {
        self.edit().is_ok_and(|mut document| document.undo())
    }

    pub fn redo(&self) -> bool {
        self.edit().is_ok_and(|mut document| document.redo())
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
        Ok(self.edit()?.save_with(path, &format.try_into()?)?)
    }

    pub fn save(&self, path: String) -> Result<(), CoreError> {
        Ok(self.edit()?.save(path)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.edit()?.to_bytes()?)
    }
}

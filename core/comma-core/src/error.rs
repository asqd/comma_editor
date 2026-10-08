#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("row {row} is out of range (rows: {rows})")]
    RowOutOfRange { row: usize, rows: usize },

    #[error("column {column} is out of range (columns: {columns})")]
    ColumnOutOfRange { column: usize, columns: usize },

    #[error("the value in row {row}, column {column} cannot be saved in {encoding}")]
    Unmappable {
        row: usize,
        column: usize,
        encoding: &'static str,
    },

    #[error("the last column cannot be deleted")]
    LastColumn,

    #[error("invalid regular expression: {0}")]
    InvalidPattern(String),

    #[error("{0} cannot be used as a delimiter")]
    InvalidDelimiter(char),

    #[error("{0}")]
    Export(String),

    /// An Excel sheet holds at most `limit` data rows below its header.
    #[error("Excel sheets hold at most {limit} rows; this table has {rows}")]
    TooManyRowsForExcel { rows: usize, limit: usize },

    #[error("{0}")]
    Formula(crate::formula::FormulaError),

    #[error("there is no column “{0}”")]
    UnknownColumn(String),

    #[error("{0}")]
    InvalidRecipe(String),

    #[error("{0}")]
    Workbook(WorkbookError),

    /// A step of a recipe failed; `step` counts from 0.
    #[error("step {}: {source}", step + 1)]
    Recipe { step: usize, source: Box<Error> },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Why an Excel workbook or one of its sheets cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkbookError {
    #[error("the file is not an Excel workbook")]
    NotAWorkbook,
    /// An OLE compound file: an `.xls` workbook, or an `.xlsx` one with a password.
    #[error("the workbook is encrypted or in the old Excel 97–2003 format (.xls)")]
    EncryptedOrLegacy,
    /// A part of the file cannot be unpacked, or its XML is broken at byte `position`.
    #[error("{part} is damaged: {detail}")]
    Damaged { part: String, position: Option<u64>, detail: String },
    /// `sheet` counts from 1.
    #[error("there is no sheet {sheet}")]
    NoSuchSheet { sheet: usize },
    #[error("“{sheet}” is a chart or macro sheet, not a table")]
    NotATable { sheet: String },
    #[error("the cells of sheet “{sheet}” are missing")]
    MissingCells { sheet: String },
}

impl From<WorkbookError> for Error {
    fn from(error: WorkbookError) -> Self {
        Self::Workbook(error)
    }
}

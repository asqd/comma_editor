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

    /// A sheet of a workbook cannot take this change and still be saved as it was.
    #[error("{0}")]
    SheetStructure(SheetStructure),

    /// A cell hidden by a merged area of a sheet cannot hold a value; the area's first cell,
    /// at `row` and `column`, does.
    #[error("the cell is inside a merged area, whose value is in its first cell")]
    MergedCell { row: usize, column: usize },

    /// A step of a recipe failed; `step` counts from 0.
    #[error("step {}: {source}", step + 1)]
    Recipe { step: usize, source: Box<Error> },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Changes to the shape of a sheet that saving cannot keep yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SheetStructure {
    /// Rows cannot be inserted, deleted or moved: formulas, merged cells, conditional formats,
    /// validations, tables or charts refer to them.
    #[error("rows of this sheet cannot be inserted, deleted or moved: other parts of the workbook refer to them")]
    RowsFixed,
    /// Columns of a workbook cannot be inserted, deleted or moved yet; they can be added at
    /// the end.
    #[error("columns of a workbook cannot be inserted, deleted or moved")]
    ColumnsFixed,
}

/// Why an Excel workbook or one of its sheets cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkbookError {
    #[error("the file is not an Excel workbook")]
    NotAWorkbook,
    /// A web page with a table, saved with a workbook's extension, as many exports are.
    #[error("the file is a web page, not an Excel workbook")]
    WebPage,
    /// An Excel 2003 XML spreadsheet saved with a workbook's extension.
    #[error("the file is an Excel 2003 XML spreadsheet")]
    Xml2003,
    /// The workbook is protected with a password.
    #[error("the workbook is encrypted")]
    Encrypted,
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
    /// Excel 97–2003 workbooks are read, not saved.
    #[error("Excel 97–2003 workbooks can only be read")]
    ReadOnly,
}

impl From<WorkbookError> for Error {
    fn from(error: WorkbookError) -> Self {
        Self::Workbook(error)
    }
}

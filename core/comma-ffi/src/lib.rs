//! UniFFI bindings of `comma-core` for the macOS app.
//!
//! Indices cross the boundary as `u64`; rows include the header row, if there is one.

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
        }
    }
}

#[derive(uniffi::Enum)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
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
    pub unterminated_quote: bool,
}

#[derive(uniffi::Object)]
pub struct CoreDocument {
    inner: Mutex<core::Document>,
}

impl CoreDocument {
    fn wrap(document: core::Document) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(document),
        })
    }

    fn lock(&self) -> MutexGuard<'_, core::Document> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
            line_ending: match info.dialect.line_ending {
                core::LineEnding::Lf => LineEnding::Lf,
                core::LineEnding::CrLf => LineEnding::CrLf,
                core::LineEnding::Cr => LineEnding::Cr,
            },
            quote_style: match info.dialect.quote_style {
                core::QuoteStyle::Minimal => QuoteStyle::Minimal,
                core::QuoteStyle::Always => QuoteStyle::Always,
            },
            has_header: info.has_header,
            trailing_newline: info.trailing_newline,
            row_count: info.row_count as u64,
            column_count: info.column_count as u64,
            unterminated_quote: info.unterminated_quote,
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
        Ok(self.lock().set_cell(row as usize, column as usize, &value)?)
    }

    pub fn insert_rows(&self, at: u64, count: u64) -> Result<(), CoreError> {
        Ok(self.lock().insert_rows(at as usize, count as usize)?)
    }

    pub fn delete_rows(&self, rows: Vec<u64>) -> Result<(), CoreError> {
        let rows: Vec<usize> = rows.into_iter().map(|row| row as usize).collect();
        Ok(self.lock().delete_rows(&rows)?)
    }

    pub fn insert_column(&self, at: u64) -> Result<(), CoreError> {
        Ok(self.lock().insert_column(at as usize)?)
    }

    pub fn delete_column(&self, at: u64) -> Result<(), CoreError> {
        Ok(self.lock().delete_column(at as usize)?)
    }

    pub fn set_has_header(&self, has_header: bool) {
        self.lock().set_has_header(has_header);
    }

    pub fn undo(&self) -> bool {
        self.lock().undo()
    }

    pub fn redo(&self) -> bool {
        self.lock().redo()
    }

    pub fn save(&self, path: String) -> Result<(), CoreError> {
        Ok(self.lock().save(path)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.lock().to_bytes()?)
    }
}

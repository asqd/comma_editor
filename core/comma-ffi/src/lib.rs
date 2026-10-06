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
    #[error("{message}")]
    InvalidPattern { message: String },
    #[error("{message}")]
    InvalidDelimiter { message: String },
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
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1251,
    Koi8R,
    Windows1252,
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

    // Visible rows: data rows as the current sort and filters show them.

    pub fn view_spec(&self) -> ViewSpec {
        self.lock().view_spec().into()
    }

    pub fn set_view(&self, spec: ViewSpec) -> Result<(), CoreError> {
        Ok(self.lock().set_view(spec.into())?)
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
        Ok(self.lock().apply_sort()?)
    }

    pub fn find(
        &self,
        query: SearchQuery,
        from: Option<CellPosition>,
        forward: bool,
    ) -> Result<Option<CellPosition>, CoreError> {
        let from = from.map(|cell| (cell.row as usize, cell.column as usize));
        let found = self.lock().find(&query.into(), from, forward)?;
        Ok(found.map(|(row, column)| CellPosition {
            row: row as u64,
            column: column as u64,
        }))
    }

    pub fn count_matches(&self, query: SearchQuery) -> Result<u64, CoreError> {
        Ok(self.lock().count_matches(&query.into())? as u64)
    }

    /// Replaces the matches in one visible cell; `false` if it does not match.
    pub fn replace_in_cell(
        &self,
        visible_row: u64,
        column: u64,
        query: SearchQuery,
        replacement: String,
    ) -> Result<bool, CoreError> {
        Ok(self.lock().replace_in_cell(
            visible_row as usize,
            column as usize,
            &query.into(),
            &replacement,
        )?)
    }

    /// Replaces every match in the visible rows; returns the number of changed cells.
    pub fn replace_all(&self, query: SearchQuery, replacement: String) -> Result<u64, CoreError> {
        Ok(self.lock().replace_all(&query.into(), &replacement)? as u64)
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
        Ok(self.lock().save_with(path, &format.try_into()?)?)
    }

    pub fn save(&self, path: String) -> Result<(), CoreError> {
        Ok(self.lock().save(path)?)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.lock().to_bytes()?)
    }
}

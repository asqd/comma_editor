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
}

pub type Result<T> = std::result::Result<T, Error>;

//! Core of CommaEditor: a CSV document model that never touches what the user did not edit.
//!
//! The original file bytes stay in memory. Unchanged records are written back byte for byte,
//! so `open → save` without edits reproduces the file exactly, and an edit changes only the
//! fields that were edited.

mod dialect;
mod document;
mod encoding;
mod error;
mod history;
mod parser;
mod search;
mod view;
mod writer;

pub use dialect::{Dialect, LineEnding, QuoteStyle};
pub use document::{Document, Info};
pub use encoding::TextEncoding;
pub use error::{Error, Result};
pub use parser::Terminator;
pub use search::SearchQuery;
pub use view::{Filter, FilterOp, SortKey, ViewSpec};
pub use writer::SaveFormat;

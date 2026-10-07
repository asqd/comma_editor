//! Core of CommaEditor: a CSV document model that never touches what the user did not edit.
//!
//! The original file bytes stay in memory. Unchanged records are written back byte for byte,
//! so `open → save` without edits reproduces the file exactly, and an edit changes only the
//! fields that were edited.

mod bytes;
mod columns;
mod dialect;
mod document;
mod encoding;
mod export;
mod formula;
mod garbled;
mod error;
mod history;
mod parser;
mod profile;
mod range;
mod repair;
mod search;
mod view;
mod writer;

pub use columns::TextTransform;
pub use dialect::{Dialect, LineEnding, QuoteStyle};
pub use document::{Document, FilePreview, IndexChunk, Indexer, Info, OpenOptions, ViewTask};
pub use encoding::TextEncoding;
pub use export::ExportFormat;
pub use garbled::fix_garbled;
pub use formula::{FUNCTIONS, FormulaError, FormulaErrorKind, FormulaReport};
pub use error::{Error, Result};
pub use parser::Terminator;
pub use profile::{ColumnProfile, Issue, ValueKind, classify};
pub use range::{CellRange, RangeStats, parse_pasted_text};
pub use repair::{Fix, Problem, ProblemKind, RepairOption, RepairReport};
pub use search::SearchQuery;
pub use view::{Filter, FilterOp, SortKey, ViewSpec};
pub use writer::SaveFormat;

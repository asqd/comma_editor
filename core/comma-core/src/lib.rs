//! Core of CommaEditor: a CSV document model that never touches what the user did not edit.
//!
//! The original file bytes stay in memory. Unchanged records are written back byte for byte,
//! so `open → save` without edits reproduces the file exactly, and an edit changes only the
//! fields that were edited.
//!
//! Sheets of Excel workbooks open as documents too; see `Workbook`.

mod bytes;
mod columns;
mod dialect;
mod diff;
mod document;
mod edit;
mod encoding;
mod export;
mod formula;
mod garbled;
mod error;
mod history;
mod parser;
mod profile;
mod range;
mod recipe;
mod repair;
mod search;
mod view;
mod writer;
mod xlsx;

pub use columns::TextTransform;
pub use dialect::{Dialect, LineEnding, QuoteStyle};
pub use diff::{Diff, DiffCell, DiffColumn, DiffCounts, DiffEntry, DiffOptions, DiffStatus};
pub use document::{Document, FilePreview, IndexChunk, Indexer, Info, OpenOptions, ViewTask};
pub use edit::{Edit, EditOutcome, EditTask, Progress};
pub use encoding::TextEncoding;
pub use export::ExportFormat;
pub use garbled::fix_garbled;
pub use formula::{FUNCTIONS, FormulaError, FormulaErrorKind, FormulaReport};
pub use error::{Error, Result, SheetStructure, WorkbookError};
pub use parser::Terminator;
pub use profile::{ColumnProfile, Issue, ValueKind, classify};
pub use range::{CellRange, RangeStats, parse_pasted_text};
pub use recipe::{Action, Recipe, RecipeStep, RowScope, StepFilter, StepSort};
pub use repair::{Fix, Problem, ProblemKind, RepairOption, RepairReport};
pub use search::SearchQuery;
pub use view::{Filter, FilterOp, SortKey, ViewSpec};
pub use writer::SaveFormat;
pub use xlsx::{
    Axis, ColumnLayout, DisplayLocale, Recalculation, Sheet, SheetEntry, SheetKind, SheetLayout, SheetRange, SheetVisibility,
    Workbook, WorkbookFormat,
    cell_name, column_letters, localize_formula,
};

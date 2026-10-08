//! Recipes: changes recorded as steps that name columns by their headers, so the same cleanup
//! can be repeated on the next export of a table even if its columns moved, and written out
//! as pandas code. A step made while a filter or sort was on keeps them: it works on the rows
//! the table showed, in that order.
//!
//! A recipe runs on a copy of the document, step by step, and the result replaces the rows as
//! one undoable step; a step that fails stops the recipe and changes nothing.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::columns::TextTransform;
use crate::document::Document;
use crate::edit::{Change, Edit, EditOutcome, Prepared, Progress, drop_later};
use crate::error::{Error, Result};
use crate::range::CellRange;
use crate::repair::ProblemKind;
use crate::search::SearchQuery;
use crate::view::{Filter, FilterOp, SortKey, ViewSpec};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(default)]
    pub name: String,
    pub steps: Vec<RecipeStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeStep {
    #[serde(flatten)]
    pub action: Action,
    /// The rows the step works on, as the table showed them when it was recorded; every row
    /// in file order when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<RowScope>,
}

/// What a step does. Columns are named by their header, or `#3` for the third column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Replace in `columns` (all when empty).
    Replace {
        find: String,
        replacement: String,
        #[serde(default)]
        case_sensitive: bool,
        #[serde(default)]
        whole_cell: bool,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        columns: Vec<String>,
    },
    Transform { columns: Vec<String>, transform: TextTransform },
    FillEmpty {
        columns: Vec<String>,
        from_above: bool,
        #[serde(default)]
        value: String,
    },
    /// Fix garbled text in `columns`, or in the whole file with the header when empty.
    FixGarbled {
        #[serde(default)]
        columns: Vec<String>,
    },
    /// A computed column after `after` (first when `None`).
    AddColumn {
        name: String,
        formula: String,
        #[serde(default)]
        after: Option<String>,
    },
    SetColumn { column: String, formula: String },
    SplitColumn {
        column: String,
        separator: String,
        #[serde(default)]
        regex: bool,
    },
    MergeColumns { columns: Vec<String>, separator: String },
    RemoveDuplicates {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        ignore_case_and_spaces: bool,
    },
    /// An empty column after `after` (first when `None`).
    InsertColumn {
        #[serde(default)]
        after: Option<String>,
    },
    DeleteColumn { column: String },
    RenameColumn { column: String, to: String },
    /// Move a column to stand after `after` (first when `None`).
    MoveColumn {
        column: String,
        #[serde(default)]
        after: Option<String>,
    },
    /// Reorder the rows of the file.
    Sort { keys: Vec<StepSort> },
    /// Delete the rows the step's filters select.
    DeleteRows,
    /// Fix every broken row of a kind as the repair panel recommends.
    FixRows { problem: ProblemKind },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowScope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<StepFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<StepSort>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepFilter {
    /// Any column when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub op: FilterOp,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepSort {
    pub column: String,
    #[serde(default = "ascending")]
    pub ascending: bool,
}

fn ascending() -> bool {
    true
}

impl Recipe {
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|error| Error::InvalidRecipe(error.to_string()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a recipe is plain data")
    }
}

impl RecipeStep {
    fn new(action: Action, rows: Option<RowScope>) -> Self {
        Self { action, rows }
    }

    /// What the step does, for a list: a kind and its arguments, which the app puts in words.
    pub fn describe(&self) -> (&'static str, Vec<String>) {
        let list = |columns: &[String]| columns.join(", ");
        match &self.action {
            Action::Replace { find, replacement, columns, .. } => ("replace", vec![find.clone(), replacement.clone(), list(columns)]),
            Action::Transform { columns, transform } => {
                let name = match transform {
                    TextTransform::Trim => "trim",
                    TextTransform::CollapseSpaces => "collapse_spaces",
                    TextTransform::Uppercase => "uppercase",
                    TextTransform::Lowercase => "lowercase",
                    TextTransform::Capitalize => "capitalize",
                };
                (name, vec![list(columns)])
            }
            Action::FillEmpty { columns, from_above: true, .. } => ("fill_from_above", vec![list(columns)]),
            Action::FillEmpty { columns, value, .. } => ("fill_empty", vec![list(columns), value.clone()]),
            Action::FixGarbled { columns } => ("fix_garbled", vec![list(columns)]),
            Action::AddColumn { name, formula, .. } => ("add_column", vec![name.clone(), formula.clone()]),
            Action::SetColumn { column, formula } => ("set_column", vec![column.clone(), formula.clone()]),
            Action::SplitColumn { column, separator, .. } => ("split_column", vec![column.clone(), separator.clone()]),
            Action::MergeColumns { columns, separator } => ("merge_columns", vec![list(columns), separator.clone()]),
            Action::RemoveDuplicates { columns, .. } => ("remove_duplicates", vec![list(columns)]),
            Action::InsertColumn { after } => ("insert_column", vec![after.clone().unwrap_or_default()]),
            Action::DeleteColumn { column } => ("delete_column", vec![column.clone()]),
            Action::RenameColumn { column, to } => ("rename_column", vec![column.clone(), to.clone()]),
            Action::MoveColumn { column, after } => ("move_column", vec![column.clone(), after.clone().unwrap_or_default()]),
            Action::Sort { keys } => ("sort", vec![keys.iter().map(|key| key.column.clone()).collect::<Vec<_>>().join(", ")]),
            Action::DeleteRows => ("delete_rows", Vec::new()),
            Action::FixRows { problem } => {
                let name = match problem {
                    ProblemKind::UnclosedQuote => "unclosed_quote",
                    ProblemKind::StrayQuote => "stray_quote",
                    ProblemKind::MissingFields => "missing_fields",
                    ProblemKind::ExtraFields => "extra_fields",
                };
                ("fix_rows", vec![name.to_owned()])
            }
        }
    }

    /// The filter conditions of the step: column (any when `None`), operator and value.
    pub fn filter_conditions(&self) -> Vec<(Option<String>, &'static str, String)> {
        let filters = self.rows.as_ref().map(|rows| rows.filters.as_slice()).unwrap_or_default();
        filters
            .iter()
            .map(|filter| {
                let op = match filter.op {
                    FilterOp::Contains => "contains",
                    FilterOp::NotContains => "not_contains",
                    FilterOp::Equals => "equals",
                    FilterOp::NotEquals => "not_equals",
                    FilterOp::StartsWith => "starts_with",
                    FilterOp::EndsWith => "ends_with",
                    FilterOp::Matches => "matches",
                    FilterOp::IsEmpty => "is_empty",
                    FilterOp::IsNotEmpty => "is_not_empty",
                    FilterOp::GreaterThan => "greater_than",
                    FilterOp::LessThan => "less_than",
                };
                (filter.column.clone(), op, filter.value.clone())
            })
            .collect()
    }

    /// Whether the step works on the rows a filter selected.
    pub fn is_filtered(&self) -> bool {
        self.rows.as_ref().is_some_and(|rows| !rows.filters.is_empty())
    }
}

// MARK: Recording

impl Document {
    /// How a recipe names a column: its header, or `#3` without a header row or for an empty
    /// or repeated header.
    pub(crate) fn column_ref(&self, column: usize) -> String {
        if self.has_header() && self.row_count() > 0 {
            let name = self.value_at(0, column);
            let unique = (0..self.column_count()).filter(|&c| self.value_at(0, c) == name).count() == 1;
            if !name.trim().is_empty() && unique {
                return name.into_owned();
            }
        }
        format!("#{}", column + 1)
    }

    /// The column a recipe names: by header, then by header ignoring case and surrounding
    /// spaces, then as `#3`.
    pub(crate) fn resolve_column(&self, name: &str) -> Result<usize> {
        let headers: Vec<String> = if self.has_header() && self.row_count() > 0 {
            (0..self.column_count()).map(|c| self.value_at(0, c).into_owned()).collect()
        } else {
            Vec::new()
        };
        headers
            .iter()
            .position(|header| header == name)
            .or_else(|| {
                let wanted = name.trim().to_lowercase();
                headers.iter().position(|header| header.trim().to_lowercase() == wanted)
            })
            .or_else(|| {
                name.strip_prefix('#')
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| n.checked_sub(1))
                    .filter(|&c| c < self.column_count())
            })
            .ok_or_else(|| Error::UnknownColumn(name.to_owned()))
    }

    fn column_refs(&self, columns: impl IntoIterator<Item = usize>) -> Vec<String> {
        columns.into_iter().map(|c| self.column_ref(c)).collect()
    }

    /// The filters and sort on now, by column names.
    fn row_scope(&self) -> Option<RowScope> {
        let spec = self.view_spec();
        if spec.is_empty() {
            return None;
        }
        Some(RowScope {
            filters: spec
                .filters
                .iter()
                .map(|filter| StepFilter {
                    column: filter.column.map(|c| self.column_ref(c)),
                    op: filter.op,
                    value: filter.value.clone(),
                    case_sensitive: filter.case_sensitive,
                })
                .collect(),
            sort: spec
                .sort
                .iter()
                .map(|key| StepSort { column: self.column_ref(key.column), ascending: key.ascending })
                .collect(),
        })
    }

    /// Whether a range takes in every visible row: only such a change repeats on another file.
    fn covers_visible_rows(&self, range: &CellRange) -> bool {
        range.top == 0 && range.bottom + 1 >= self.visible_row_count()
    }

    /// The recipe step that repeats `edit`, made before it runs; `None` for an edit of some
    /// rows only, which another file would not have.
    pub fn recipe_step_for_edit(&self, edit: &Edit) -> Option<RecipeStep> {
        let ranged = |range: &CellRange| self.covers_visible_rows(range).then(|| self.column_refs(range.columns()));
        Some(match edit {
            Edit::ReplaceAll { query, replacement } => {
                let columns = match (&query.range, query.column) {
                    (Some(range), column) => {
                        let columns = ranged(range)?;
                        match column {
                            Some(c) => vec![self.column_ref(c)],
                            None => columns,
                        }
                    }
                    (None, Some(c)) => vec![self.column_ref(c)],
                    (None, None) => Vec::new(),
                };
                RecipeStep::new(
                    Action::Replace {
                        find: query.text.clone(),
                        replacement: replacement.clone(),
                        case_sensitive: query.case_sensitive,
                        whole_cell: query.whole_cell,
                        regex: query.regex,
                        columns,
                    },
                    self.row_scope(),
                )
            }
            Edit::Transform { range, transform } => {
                RecipeStep::new(Action::Transform { columns: ranged(range)?, transform: *transform }, self.row_scope())
            }
            Edit::FillEmpty { range, from_above, value } => RecipeStep::new(
                Action::FillEmpty { columns: ranged(range)?, from_above: *from_above, value: value.clone() },
                self.row_scope(),
            ),
            Edit::FixGarbled { range: None } => RecipeStep::new(Action::FixGarbled { columns: Vec::new() }, None),
            Edit::FixGarbled { range: Some(range) } => {
                RecipeStep::new(Action::FixGarbled { columns: ranged(range)? }, self.row_scope())
            }
            Edit::AddFormulaColumn { formula, name, at } => RecipeStep::new(
                Action::AddColumn {
                    name: name.clone(),
                    formula: formula.clone(),
                    after: at.checked_sub(1).map(|c| self.column_ref(c)),
                },
                None,
            ),
            Edit::FillWithFormula { formula, column } => RecipeStep::new(
                Action::SetColumn { column: self.column_ref(*column), formula: formula.clone() },
                self.row_scope(),
            ),
            Edit::SplitColumn { column, separator, regex } => RecipeStep::new(
                Action::SplitColumn { column: self.column_ref(*column), separator: separator.clone(), regex: *regex },
                None,
            ),
            Edit::MergeColumns { columns, separator } => RecipeStep::new(
                Action::MergeColumns { columns: self.column_refs(columns.iter().copied()), separator: separator.clone() },
                None,
            ),
            Edit::RemoveDuplicates { columns, normalize } => RecipeStep::new(
                Action::RemoveDuplicates {
                    columns: self.column_refs(columns.iter().copied()),
                    ignore_case_and_spaces: *normalize,
                },
                self.row_scope(),
            ),
            Edit::Recipe(_) => return None,
        })
    }

    pub fn recipe_step_for_insert_column(&self, at: usize) -> RecipeStep {
        RecipeStep::new(Action::InsertColumn { after: at.checked_sub(1).map(|c| self.column_ref(c)) }, None)
    }

    pub fn recipe_step_for_delete_column(&self, at: usize) -> RecipeStep {
        RecipeStep::new(Action::DeleteColumn { column: self.column_ref(at) }, None)
    }

    /// A change of the header row renames a column; other cells are not repeated.
    pub fn recipe_step_for_set_cell(&self, row: usize, column: usize, value: &str) -> Option<RecipeStep> {
        (row == 0 && self.has_header()).then(|| {
            RecipeStep::new(Action::RenameColumn { column: self.column_ref(column), to: value.to_owned() }, None)
        })
    }

    pub fn recipe_step_for_move_column(&self, from: usize, to: usize) -> RecipeStep {
        let mut order: Vec<usize> = (0..self.column_count()).filter(|&c| c != from).collect();
        order.insert(to.min(order.len()), from);
        let position = order.iter().position(|&c| c == from).unwrap_or(0);
        RecipeStep::new(
            Action::MoveColumn {
                column: self.column_ref(from),
                after: position.checked_sub(1).map(|p| self.column_ref(order[p])),
            },
            None,
        )
    }

    pub fn recipe_step_for_apply_sort(&self) -> Option<RecipeStep> {
        let scope = self.row_scope()?;
        (!scope.sort.is_empty()).then(|| RecipeStep::new(Action::Sort { keys: scope.sort }, None))
    }

    /// Deleting every row a filter shows repeats as deleting the rows the filter selects;
    /// deleting chosen rows does not repeat.
    pub fn recipe_step_for_delete_rows(&self, rows: &[usize]) -> Option<RecipeStep> {
        let scope = self.row_scope()?;
        let mut rows = rows.to_vec();
        rows.sort_unstable();
        rows.dedup();
        let all = rows.len() == self.visible_row_count()
            && (0..self.visible_row_count()).all(|row| self.visible_record(row).is_some_and(|r| rows.binary_search(&r).is_ok()));
        (!scope.filters.is_empty() && all)
            .then(|| RecipeStep::new(Action::DeleteRows, Some(RowScope { filters: scope.filters, sort: Vec::new() })))
    }

    pub fn recipe_step_for_fix_rows(&self, problem: ProblemKind) -> RecipeStep {
        RecipeStep::new(Action::FixRows { problem }, None)
    }
}

// MARK: Running

impl Document {
    /// Runs a recipe on a copy of this document; the result replaces every row as one step.
    /// `Ok(None)` when cancelled.
    pub(crate) fn prepare_recipe(&self, recipe: &Recipe, progress: &Progress) -> Result<Option<Prepared>> {
        let mut work = self.snapshot();
        // The copy keeps only the last step's history: nothing undoes it.
        work.set_undo_memory_limit(0);
        work.reset_view();
        let count = recipe.steps.len();
        for (index, step) in recipe.steps.iter().enumerate() {
            progress.begin_part(index, count);
            let finished = work
                .run_step(step, progress)
                .map_err(|error| Error::Recipe { step: index, source: Box::new(error) })?;
            if !finished {
                drop_later(work);
                return Ok(None);
            }
        }
        progress.begin_part(count, count.max(1));
        if Arc::ptr_eq(&work.records, &self.records) {
            return Ok(Some(Prepared::nothing()));
        }
        let records = std::mem::take(&mut work.records);
        let records = Arc::try_unwrap(records).unwrap_or_else(|shared| (*shared).clone());
        let columns_after = work.column_count();
        drop_later(work);
        Ok(Some(Prepared {
            change: Change::All { records, columns_after },
            outcome: EditOutcome { changed: false, count, report: None },
        }))
    }

    /// Runs one step; `Ok(false)` when cancelled.
    fn run_step(&mut self, step: &RecipeStep, progress: &Progress) -> Result<bool> {
        match &step.rows {
            Some(scope) => {
                let spec = ViewSpec {
                    filters: scope
                        .filters
                        .iter()
                        .map(|filter| {
                            Ok(Filter {
                                column: filter.column.as_deref().map(|name| self.resolve_column(name)).transpose()?,
                                op: filter.op,
                                value: filter.value.clone(),
                                case_sensitive: filter.case_sensitive,
                            })
                        })
                        .collect::<Result<_>>()?,
                    sort: self.sort_keys(&scope.sort)?,
                };
                self.set_view(spec)?;
            }
            None => self.reset_view(),
        }
        let columns = |document: &Self, names: &[String]| -> Result<Vec<usize>> {
            names.iter().map(|name| document.resolve_column(name)).collect()
        };
        // The whole of one column over the rows shown; `None` when no row is shown.
        let column_range = |document: &Self, column: usize| -> Option<CellRange> {
            let rows = document.visible_row_count();
            (rows > 0).then_some(CellRange { top: 0, left: column, bottom: rows - 1, right: column })
        };
        let mut edits: Vec<Edit> = Vec::new();
        match &step.action {
            Action::Replace { find, replacement, case_sensitive, whole_cell, regex, columns: names } => {
                let query = |column: Option<usize>| Edit::ReplaceAll {
                    query: SearchQuery {
                        text: find.clone(),
                        case_sensitive: *case_sensitive,
                        whole_cell: *whole_cell,
                        regex: *regex,
                        column,
                        range: None,
                    },
                    replacement: replacement.clone(),
                };
                if names.is_empty() {
                    edits.push(query(None));
                } else {
                    edits.extend(columns(self, names)?.into_iter().map(|c| query(Some(c))));
                }
            }
            Action::Transform { columns: names, transform } => {
                for column in columns(self, names)? {
                    edits.extend(column_range(self, column).map(|range| Edit::Transform { range, transform: *transform }));
                }
            }
            Action::FillEmpty { columns: names, from_above, value } => {
                for column in columns(self, names)? {
                    edits.extend(column_range(self, column).map(|range| Edit::FillEmpty {
                        range,
                        from_above: *from_above,
                        value: value.clone(),
                    }));
                }
            }
            Action::FixGarbled { columns: names } if names.is_empty() => edits.push(Edit::FixGarbled { range: None }),
            Action::FixGarbled { columns: names } => {
                for column in columns(self, names)? {
                    edits.extend(column_range(self, column).map(|range| Edit::FixGarbled { range: Some(range) }));
                }
            }
            Action::AddColumn { name, formula, after } => {
                let at = match after {
                    Some(after) => self.resolve_column(after)? + 1,
                    None => 0,
                };
                edits.push(Edit::AddFormulaColumn { formula: formula.clone(), name: name.clone(), at });
            }
            Action::SetColumn { column, formula } => {
                edits.push(Edit::FillWithFormula { formula: formula.clone(), column: self.resolve_column(column)? });
            }
            Action::SplitColumn { column, separator, regex } => edits.push(Edit::SplitColumn {
                column: self.resolve_column(column)?,
                separator: separator.clone(),
                regex: *regex,
            }),
            Action::MergeColumns { columns: names, separator } => {
                edits.push(Edit::MergeColumns { columns: columns(self, names)?, separator: separator.clone() });
            }
            Action::RemoveDuplicates { columns: names, ignore_case_and_spaces } => edits.push(Edit::RemoveDuplicates {
                columns: columns(self, names)?,
                normalize: *ignore_case_and_spaces,
            }),
            Action::InsertColumn { after } => {
                let at = match after {
                    Some(after) => self.resolve_column(after)? + 1,
                    None => 0,
                };
                self.insert_column(at)?;
            }
            Action::DeleteColumn { column } => {
                let column = self.resolve_column(column)?;
                self.delete_column(column)?;
            }
            Action::RenameColumn { column, to } => {
                let column = self.resolve_column(column)?;
                if self.has_header() {
                    self.set_cell(0, column, to)?;
                }
            }
            Action::MoveColumn { column, after } => {
                let from = self.resolve_column(column)?;
                let to = match after {
                    // The position among the other columns, which is where `after` stands
                    // once `from` is taken out.
                    Some(after) => {
                        let after = self.resolve_column(after)?;
                        if after < from { after + 1 } else { after }
                    }
                    None => 0,
                };
                self.move_column(from, to)?;
            }
            Action::Sort { keys } => {
                let sort = self.sort_keys(keys)?;
                self.set_view(ViewSpec { sort, filters: Vec::new() })?;
                self.apply_sort()?;
            }
            Action::DeleteRows => {
                if step.is_filtered() {
                    let rows: Vec<usize> = (0..self.visible_row_count()).filter_map(|row| self.visible_record(row)).collect();
                    self.delete_rows(&rows)?;
                }
            }
            Action::FixRows { problem } => {
                self.fix_all(*problem);
            }
        }
        for edit in edits {
            match self.prepare_edit(&edit, progress)? {
                Some(prepared) => {
                    self.apply_prepared(prepared);
                }
                None => return Ok(false),
            }
        }
        Ok(!progress.is_cancelled())
    }

    fn sort_keys(&self, keys: &[StepSort]) -> Result<Vec<SortKey>> {
        keys.iter()
            .map(|key| Ok(SortKey { column: self.resolve_column(&key.column)?, ascending: key.ascending }))
            .collect()
    }
}

// MARK: pandas

impl Recipe {
    /// The recipe as a Python script with pandas that reads `input`, makes the same changes
    /// and writes `output`. Formulas are translated where CommaEditor's functions have a close
    /// Python equivalent; other steps are left as comments.
    pub fn to_pandas(&self, input: &str, output: &str, delimiter: char) -> String {
        let mut body = String::new();
        let mut needs_helpers = false;
        for (index, step) in self.steps.iter().enumerate() {
            body.push_str(&format!("\n# {}. {}\n", index + 1, step_comment(step)));
            body.push_str(&pandas_step(step, &mut needs_helpers));
        }
        let mut out = String::new();
        out.push_str(&format!("# {}\n", if self.name.is_empty() { "CommaEditor recipe" } else { &self.name }));
        out.push_str("# Made by CommaEditor. Values are read as text, as CommaEditor reads them.\n\n");
        out.push_str("import re\n\nimport pandas as pd\n\n");
        out.push_str(&format!("INPUT = {}\nOUTPUT = {}\nSEP = {}\n", py_string(input), py_string(output), py_string(&delimiter.to_string())));
        if needs_helpers {
            out.push_str(PYTHON_HELPERS);
        }
        out.push_str("\ndf = pd.read_csv(INPUT, sep=SEP, dtype=str, keep_default_na=False)\n");
        out.push_str(&body);
        out.push_str("\ndf.to_csv(OUTPUT, sep=SEP, index=False)\n");
        out
    }
}

/// Helpers that read values the way CommaEditor's formulas do.
const PYTHON_HELPERS: &str = r#"

def num(value):
    """A value as a number: "1 250,50" and "1,250.50" read as 1250.5, an empty value as 0."""
    if isinstance(value, bool):
        return float(value)
    if isinstance(value, (int, float)):
        return float(value)
    text = str(value).strip().replace(" ", "").replace(" ", "")
    if text == "":
        return 0.0
    if "," in text and "." in text:
        text = text.replace(",", "") if text.rfind(".") > text.rfind(",") else text.replace(".", "").replace(",", ".")
    elif text.count(",") == 1:
        text = text.replace(",", ".")
    return float(text)


def text(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float):
        value = round(value, 10)
        return str(int(value)) if value == int(value) else repr(value)
    return "" if value is None else str(value)


def truth(value):
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return value != 0
    return str(value).strip().lower() not in ("", "0", "false", "no", "нет")


def compare(a, b):
    try:
        x, y = num(a), num(b)
        if text(a).strip() != "" and text(b).strip() != "":
            return (x > y) - (x < y)
    except ValueError:
        pass
    x, y = text(a).lower(), text(b).lower()
    return (x > y) - (x < y)


def round_half(value, digits=0):
    scale = 10 ** int(digits)
    rounded = int(abs(value) * scale + 0.5) / scale
    return rounded if value >= 0 else -rounded
"#;

fn py_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A column for `df[...]`: its name, or its position for `#3`.
fn py_column(name: &str) -> String {
    match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()).filter(|&n| n > 0) {
        Some(n) => format!("df.columns[{}]", n - 1),
        None => py_string(name),
    }
}

fn py_columns(names: &[String]) -> String {
    if names.is_empty() {
        "list(df.columns)".to_owned()
    } else {
        format!("[{}]", names.iter().map(|name| py_column(name)).collect::<Vec<_>>().join(", "))
    }
}

fn step_comment(step: &RecipeStep) -> String {
    let quoted = |text: &str| format!("\"{text}\"");
    let columns = |columns: &[String]| if columns.is_empty() { "every column".to_owned() } else { columns.join(", ") };
    let mut comment = match &step.action {
        Action::Replace { find, replacement, columns: names, .. } => {
            format!("Replace {} with {} in {}", quoted(find), quoted(replacement), columns(names))
        }
        Action::Transform { columns: names, transform } => {
            let what = match transform {
                TextTransform::Trim => "Trim spaces",
                TextTransform::CollapseSpaces => "Remove extra spaces",
                TextTransform::Uppercase => "Upper case",
                TextTransform::Lowercase => "Lower case",
                TextTransform::Capitalize => "Capitalize words",
            };
            format!("{what} in {}", columns(names))
        }
        Action::FillEmpty { columns: names, from_above: true, .. } => format!("Fill empty cells of {} from above", columns(names)),
        Action::FillEmpty { columns: names, value, .. } => format!("Fill empty cells of {} with {}", columns(names), quoted(value)),
        Action::FixGarbled { .. } => "Fix garbled text".to_owned(),
        Action::AddColumn { name, formula, .. } => format!("Add column {} = {formula}", quoted(name)),
        Action::SetColumn { column, formula } => format!("Set {} to {formula}", quoted(column)),
        Action::SplitColumn { column, separator, .. } => format!("Split {} at {}", quoted(column), quoted(separator)),
        Action::MergeColumns { columns: names, separator } => format!("Merge {} with {}", names.join(", "), quoted(separator)),
        Action::RemoveDuplicates { columns: names, .. } => format!("Remove rows that repeat in {}", columns(names)),
        Action::InsertColumn { .. } => "Insert an empty column".to_owned(),
        Action::DeleteColumn { column } => format!("Delete column {}", quoted(column)),
        Action::RenameColumn { column, to } => format!("Rename column {} to {}", quoted(column), quoted(to)),
        Action::MoveColumn { column, after: Some(after) } => format!("Move column {} after {}", quoted(column), quoted(after)),
        Action::MoveColumn { column, after: None } => format!("Move column {} to the start", quoted(column)),
        Action::Sort { keys } => format!("Sort the rows by {}", keys.iter().map(|key| key.column.as_str()).collect::<Vec<_>>().join(", ")),
        Action::DeleteRows => "Delete the rows the filter selects".to_owned(),
        Action::FixRows { .. } => "Repair broken rows".to_owned(),
    };
    if step.is_filtered() {
        comment.push_str(", in the rows the filter selects");
    }
    comment.replace('\n', " ")
}

/// Code that sets `rows`: the index of the rows the step works on, in the order it sees them.
fn py_rows(scope: Option<&RowScope>) -> String {
    let Some(scope) = scope else {
        return "rows = df.index\n".to_owned();
    };
    let mut out = String::from("mask = pd.Series(True, index=df.index)\n");
    for filter in &scope.filters {
        let value = py_string(&filter.value);
        let case = if filter.case_sensitive { "True" } else { "False" };
        let positive = |series: &str| -> (String, bool) {
            let lower = |s: &str| if filter.case_sensitive { s.to_owned() } else { format!("{s}.str.lower()") };
            let value_cased = if filter.case_sensitive { value.clone() } else { format!("{value}.lower()") };
            match filter.op {
                FilterOp::Contains | FilterOp::NotContains => {
                    (format!("{series}.str.contains({value}, case={case}, regex=False)"), filter.op == FilterOp::NotContains)
                }
                FilterOp::Equals | FilterOp::NotEquals => {
                    (format!("({} == {value_cased})", lower(series)), filter.op == FilterOp::NotEquals)
                }
                FilterOp::StartsWith => (format!("{}.str.startswith({value_cased})", lower(series)), false),
                FilterOp::EndsWith => (format!("{}.str.endswith({value_cased})", lower(series)), false),
                FilterOp::Matches => (format!("{series}.str.contains({value}, case={case}, regex=True)"), false),
                FilterOp::IsEmpty | FilterOp::IsNotEmpty => {
                    (format!("({series}.str.strip() == \"\")"), filter.op == FilterOp::IsNotEmpty)
                }
                FilterOp::GreaterThan => (format!("(pd.to_numeric({series}, errors=\"coerce\") > float({value}))"), false),
                FilterOp::LessThan => (format!("(pd.to_numeric({series}, errors=\"coerce\") < float({value}))"), false),
            }
        };
        let condition = match &filter.column {
            Some(column) => {
                let (condition, negated) = positive(&format!("df[{}]", py_column(column)));
                if negated { format!("~{condition}") } else { condition }
            }
            None => {
                let (condition, negated) = positive("c");
                let any = format!("df.apply(lambda c: {condition}).any(axis=1)");
                if negated { format!("~{any}") } else { any }
            }
        };
        out.push_str(&format!("mask &= {condition}\n"));
    }
    if scope.sort.is_empty() {
        out.push_str("rows = df.index[mask]\n");
    } else {
        let by = scope.sort.iter().map(|key| py_column(&key.column)).collect::<Vec<_>>().join(", ");
        let ascending = scope.sort.iter().map(|key| if key.ascending { "True" } else { "False" }).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("rows = df[mask].sort_values(by=[{by}], ascending=[{ascending}], kind=\"stable\").index\n"));
    }
    out
}

fn pandas_step(step: &RecipeStep, needs_helpers: &mut bool) -> String {
    let rows = py_rows(step.rows.as_ref());
    match &step.action {
        Action::Replace { find, replacement, case_sensitive, whole_cell, regex, columns } => {
            let mut pattern = if *regex { py_string(find) } else { format!("re.escape({})", py_string(find)) };
            if *whole_cell {
                pattern = format!("\"^(?:\" + {pattern} + \")$\"");
            }
            // Literal replacements go through a function, so backslashes stay as they are.
            let replacement = if *regex {
                py_string(&rust_replacement_to_python(replacement))
            } else {
                format!("lambda _: {}", py_string(replacement))
            };
            let case = if *case_sensitive { "True" } else { "False" };
            format!(
                "{rows}for column in {}:\n    df.loc[rows, column] = df.loc[rows, column].str.replace({pattern}, {replacement}, case={case}, regex=True)\n",
                py_columns(columns)
            )
        }
        Action::Transform { columns, transform } => {
            let change = match transform {
                TextTransform::Trim => ".str.strip()",
                TextTransform::CollapseSpaces => ".str.split().str.join(\" \")",
                TextTransform::Uppercase => ".str.upper()",
                TextTransform::Lowercase => ".str.lower()",
                TextTransform::Capitalize => ".str.title()",
            };
            format!("{rows}for column in {}:\n    df.loc[rows, column] = df.loc[rows, column]{change}\n", py_columns(columns))
        }
        Action::FillEmpty { columns, from_above: true, .. } => format!(
            "{rows}for column in {}:\n    values = df.loc[rows, column]\n    df.loc[rows, column] = values.where(values.str.strip() != \"\").ffill().fillna(values)\n",
            py_columns(columns)
        ),
        Action::FillEmpty { columns, value, .. } => format!(
            "{rows}for column in {}:\n    empty = rows[(df.loc[rows, column].str.strip() == \"\").to_numpy()]\n    df.loc[empty, column] = {}\n",
            py_columns(columns),
            py_string(value)
        ),
        Action::FixGarbled { .. } => {
            "# Garbled text is fixed in CommaEditor; the ftfy package does much the same:\n# df = df.map(ftfy.fix_text)\n".to_owned()
        }
        Action::AddColumn { name, formula, after } => {
            let position = match after {
                Some(after) => format!("df.columns.get_loc({}) + 1", py_column(after)),
                None => "0".to_owned(),
            };
            match crate::formula::python_expression(formula) {
                Some(expression) => {
                    *needs_helpers = true;
                    format!(
                        "df.insert({position}, {}, df.apply(lambda r: text({expression}), axis=1), allow_duplicates=True)\n",
                        py_string(name)
                    )
                }
                None => format!(
                    "# The formula {} has no close pandas equivalent: fill the column by hand.\ndf.insert({position}, {}, \"\", allow_duplicates=True)\n",
                    py_string(formula),
                    py_string(name)
                ),
            }
        }
        Action::SetColumn { column, formula } => match crate::formula::python_expression(formula) {
            Some(expression) => {
                *needs_helpers = true;
                format!(
                    "{rows}df.loc[rows, {}] = df.loc[rows].apply(lambda r: text({expression}), axis=1)\n",
                    py_column(column)
                )
            }
            None => format!("# The formula {} has no close pandas equivalent.\n", py_string(formula)),
        },
        Action::SplitColumn { column, separator, regex } => format!(
            "column = {col}\nparts = df[column].str.split({}, regex={}, expand=True).fillna(\"\")\nif parts.shape[1] > 1:\n    parts.columns = [f\"{{column}} {{n + 1}}\" for n in range(parts.shape[1])]\n    at = df.columns.get_loc(column) + 1\n    df = pd.concat([df.iloc[:, :at], parts, df.iloc[:, at:]], axis=1)\n",
            py_string(separator),
            if *regex { "True" } else { "False" },
            col = py_column(column)
        ),
        Action::MergeColumns { columns, separator } => {
            let joiner = if separator.trim().is_empty() { " " } else { separator };
            format!(
                "columns = {}\nname = {}.join(c for c in columns if c)\nat = max(df.columns.get_loc(c) for c in columns) + 1\ndf.insert(at, name, df[columns].apply(lambda r: {}.join(v for v in r if v != \"\"), axis=1), allow_duplicates=True)\n",
                py_columns(columns),
                py_string(joiner),
                py_string(separator)
            )
        }
        Action::RemoveDuplicates { columns, ignore_case_and_spaces } => {
            let key = if *ignore_case_and_spaces {
                format!("df.loc[rows, {}].apply(lambda c: c.str.strip().str.lower())", py_columns(columns))
            } else {
                format!("df.loc[rows, {}]", py_columns(columns))
            };
            format!("{rows}repeated = {key}.duplicated(keep=\"first\")\ndf = df.drop(index=repeated[repeated].index)\n")
        }
        Action::InsertColumn { after } => {
            let position = match after {
                Some(after) => format!("df.columns.get_loc({}) + 1", py_column(after)),
                None => "0".to_owned(),
            };
            format!("df.insert({position}, \"\", \"\", allow_duplicates=True)\n")
        }
        Action::DeleteColumn { column } => format!("df = df.drop(columns=[{}])\n", py_column(column)),
        Action::RenameColumn { column, to } => format!("df = df.rename(columns={{{}: {}}})\n", py_column(column), py_string(to)),
        Action::MoveColumn { column, after } => {
            let position = match after {
                Some(after) => format!("order.index({}) + 1", py_column(after)),
                None => "0".to_owned(),
            };
            format!(
                "order = list(df.columns)\ncolumn = {}\norder.remove(column)\norder.insert({position}, column)\ndf = df[order]\n",
                py_column(column)
            )
        }
        Action::Sort { keys } => {
            let by = keys.iter().map(|key| py_column(&key.column)).collect::<Vec<_>>().join(", ");
            let ascending = keys.iter().map(|key| if key.ascending { "True" } else { "False" }).collect::<Vec<_>>().join(", ");
            format!(
                "# CommaEditor sorts numbers and dates by value; this sorts the text.\ndf = df.sort_values(by=[{by}], ascending=[{ascending}], kind=\"stable\").reset_index(drop=True)\n"
            )
        }
        Action::DeleteRows => format!("{rows}df = df.drop(index=rows)\n"),
        Action::FixRows { .. } => "# Broken rows are repaired in CommaEditor before the file reaches pandas.\n".to_owned(),
    }
}

/// `$1` and `${name}` in a replacement as Python writes them: `\g<1>`, `\g<name>`.
fn rust_replacement_to_python(replacement: &str) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '$' if chars.peek() == Some(&'$') => {
                chars.next();
                out.push('$');
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push_str(&format!("\\g<{name}>"));
            }
            '$' if chars.peek().is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') => {
                let mut name = String::new();
                while let Some(&c) = chars.peek().filter(|c| c.is_ascii_alphanumeric() || **c == '_') {
                    name.push(c);
                    chars.next();
                }
                out.push_str(&format!("\\g<{name}>"));
            }
            c => out.push(c),
        }
    }
    out
}

//! Formulas of workbooks: reading them, computing them, and computing again the ones that
//! depend on edited cells.
//!
//! Formulas the file computed and nothing edited depends on keep the results Excel stored:
//! computing them here could only show where this engine and Excel differ. A formula is
//! computed again when it reads, directly or through other formulas, a row edited since the
//! workbook was opened, or when it was entered here. A file that asks to compute every
//! formula on opening (written by a program, with no results or zeros) has each sheet's
//! formulas computed the first time the sheet is. Cells it reads give it the values the
//! file stores (not the rounded ones shown), or what was typed into them.

pub(crate) mod eval;
pub(crate) mod parse;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use self::eval::{Cells, Evaluator, Unsupported, Value};
use self::parse::{Area, BinaryOp, Expr, TableRef, TableRows, UnaryOp};
use super::numfmt::{DisplayLocale, NumberFormat};
use super::sheet::{RawCell, RawSheet, RawValue};
use super::{Source, TableDef, Workbook};
use crate::document::{Computed, Document, Field, FormulaField, FormulaResult};
use crate::error::Result;
use crate::formula::FormulaError;

/// A formula as Excel shows it with a decimal comma, and with Russian names when asked.
pub fn localize_formula(formula: &str, russian_names: bool) -> String {
    parse::localize(formula, russian_names)
}

/// A formula as a person typed it after `=`, written the way files keep it.
pub(crate) fn normalize(text: &str) -> std::result::Result<String, FormulaError> {
    parse::normalize(text)
}

/// What computing lasts across edits: values the file stores, what edited rows held, and the
/// formulas read.
#[derive(Default)]
pub(crate) struct CalcState {
    raw: HashMap<usize, Arc<RawSheet>>,
    /// Fingerprints of the edited rows when formulas were last computed, by sheet: a row whose
    /// fingerprint differs now, or that is no longer edited, as after an undo, changed since.
    /// Inserting, deleting or moving rows forgets a sheet's, so all its edited rows count.
    pub(crate) entered: HashMap<usize, HashMap<u32, u64>>,
    /// Formulas read so far, by text, so that computing again does not read them anew.
    parsed: HashMap<String, Option<Arc<Expr>>>,
    /// Sheets whose every formula was computed, in a file that asks for it on opening.
    computed: HashSet<usize>,
}

/// What computing again did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recalculation {
    /// Sheets whose shown values changed.
    pub sheets: Vec<usize>,
    /// Formulas that read themselves through other cells; they show 0, as in Excel.
    pub circular: usize,
    /// Formulas that need a function, a defined name or an array CommaEditor does not compute;
    /// they keep the result the file has, and Excel computes them when it opens the file.
    pub not_computed: usize,
}

struct Formula {
    sheet: usize,
    row: u32,
    column: u32,
    expr: Option<Arc<Expr>>,
    /// The cells it reads.
    reads: Vec<(usize, Area)>,
    entered: bool,
    /// The first row and column of the range an array formula fills.
    array: Option<(u32, u32)>,
}

impl Workbook {
    /// Computes again the formulas of the given sheets that depend on cells edited since the
    /// workbook was opened, and the formulas entered here. `now` is the date and time as a
    /// serial number, for TODAY and NOW.
    pub fn recalculate(&self, sheets: &mut [(usize, &mut Document)], now: f64) -> Result<Recalculation> {
        let Source::Xlsx { styles, .. } = &self.source else {
            return Ok(Recalculation::default());
        };
        let Some(locale) = sheets.iter().find_map(|(_, document)| document.sheet.as_ref().map(|sheet| sheet.locale.clone())) else {
            return Ok(Recalculation::default());
        };
        let mut state = self.calc.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Rows changed since formulas were last computed.
        let mut touched: HashMap<usize, BTreeSet<u32>> = HashMap::new();
        for (index, document) in sheets.iter() {
            let before = state.entered.remove(index).unwrap_or_default();
            let mut now = HashMap::with_capacity(before.len());
            let mut rows = BTreeSet::new();
            for row in (0..document.row_count()).filter(|&row| document.row_edited(row)) {
                let fingerprint = document.row_fingerprint(row);
                if before.get(&(row as u32)) != Some(&fingerprint) {
                    rows.insert(row as u32);
                }
                now.insert(row as u32, fingerprint);
            }
            rows.extend(before.keys().filter(|row| !now.contains_key(row)));
            state.entered.insert(*index, now);
            touched.insert(*index, rows);
        }
        let formats = styles.formats(&locale);
        let mut shown_sheets = self.show_entered(&mut state, sheets, &touched, &formats, &locale);
        let has_formulas = sheets.iter().any(|(_, document)| document.formula_cells().next().is_some());
        if !has_formulas {
            return Ok(Recalculation { sheets: shown_sheets, ..Recalculation::default() });
        }

        // Every formula of the sheets, read once per text.
        let names: Vec<String> = self.sheets.iter().map(|sheet| sheet.name.to_lowercase()).collect();
        let defined = self.defined_names();
        let tables = self.current_tables(sheets);
        let tables = tables.as_slice();
        let parsed = &mut state.parsed;
        let mut formulas = Vec::new();
        for (index, document) in sheets.iter() {
            for (row, column, cell) in document.formula_cells() {
                let expr = match parsed.get(&cell.formula) {
                    Some(expr) => expr.clone(),
                    None => {
                        let expr = parse::parse(&cell.formula).ok().map(Arc::new);
                        parsed.insert(cell.formula.clone(), expr.clone());
                        expr
                    }
                };
                let mut reads = Vec::new();
                if let Some(expr) = &expr {
                    let place = Place { sheet: *index, row: row as u32, column: column as u32 };
                    references(expr, place, &names, &defined, tables, 0, &mut reads);
                }
                let range = cell.array.as_deref().and_then(super::refs::parse_range);
                // An array formula filling several cells keeps its results: the other cells hold
                // only values, which computing here would leave behind. Excel computes it.
                let expr = if range.is_some_and(|(first, last)| first != last) { None } else { expr };
                let array = range.map(|(first, _)| first);
                formulas.push(Formula { sheet: *index, row: row as u32, column: column as u32, expr, reads, entered: !cell.from_file, array });
            }
        }
        // Formulas no longer in the sheets go once there are many of them.
        if parsed.len() > 2 * formulas.len() + 1024 {
            let texts: HashSet<&str> = sheets.iter().flat_map(|(_, document)| document.formula_cells().map(|(_, _, cell)| cell.formula.as_str())).collect();
            parsed.retain(|text, _| texts.contains(text.as_str()));
        }

        // Formulas to compute: entered ones, those reading edited rows, then those reading
        // formulas to compute, until no more are found.
        let mut marked: Vec<bool> = formulas
            .iter()
            .map(|formula| formula.entered || (self.computes_on_load && !state.computed.contains(&formula.sheet)))
            .collect();
        let mut dirty: HashMap<usize, BTreeMap<u32, BTreeSet<u32>>> = HashMap::new();
        for (formula, _) in formulas.iter().zip(&marked).filter(|(_, marked)| **marked) {
            dirty.entry(formula.sheet).or_default().entry(formula.row).or_default().insert(formula.column);
        }
        loop {
            let mut found = false;
            for (index, formula) in formulas.iter().enumerate() {
                if marked[index] {
                    continue;
                }
                let reads_changed = formula.reads.iter().any(|(sheet, area)| {
                    touched.get(sheet).is_some_and(|rows| rows.range(area.top..=area.bottom).next().is_some())
                        || dirty.get(sheet).is_some_and(|cells| {
                            cells.range(area.top..=area.bottom).any(|(_, columns)| columns.range(area.left..=area.right).next().is_some())
                        })
                });
                if reads_changed {
                    marked[index] = true;
                    found = true;
                    dirty.entry(formula.sheet).or_default().entry(formula.row).or_default().insert(formula.column);
                }
            }
            if !found {
                break;
            }
        }
        if !marked.iter().any(|marked| *marked) {
            return Ok(Recalculation { sheets: shown_sheets, ..Recalculation::default() });
        }

        let documents: HashMap<usize, &Document> = sheets.iter().map(|(index, document)| (*index, &**document)).collect();
        let positions: HashMap<(usize, u32, u32), usize> = formulas
            .iter()
            .enumerate()
            .filter(|(index, _)| marked[*index])
            .map(|(index, formula)| ((formula.sheet, formula.row, formula.column), index))
            .collect();
        let mut model = Model {
            workbook: self,
            names: &defined,
            tables,
            documents,
            raw: &mut state.raw,
            formats: &formats,
            locale: &locale,
            formulas: &formulas,
            positions,
            results: vec![None; formulas.len()],
            busy: vec![false; formulas.len()],
            stack: Vec::new(),
            in_circle: vec![false; formulas.len()],
            circular: 0,
            now,
            failed: None,
        };
        // Long chains of formulas, such as running totals, are computed by recursion; it runs
        // on a thread with room for it.
        let order: Vec<usize> = (0..formulas.len()).filter(|&index| marked[index]).collect();
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(512 << 20)
                .spawn_scoped(scope, || {
                    for &index in &order {
                        // Kept in `results`.
                        let _ = model.result(index);
                    }
                })
                .expect("a thread for computing")
                .join()
                .expect("computing does not panic");
        });
        if let Some(error) = model.failed.take() {
            drop(model);
            // Edited rows count as changed again next time.
            for (index, _) in sheets.iter() {
                state.entered.remove(index);
            }
            return Err(error);
        }

        // Results shown in each cell's format, written where they changed.
        let mut report = Recalculation { circular: model.circular, ..Recalculation::default() };
        let mut updates: HashMap<usize, Vec<FormulaResult>> = HashMap::new();
        for &index in &order {
            let formula = &formulas[index];
            let Some(Ok(value)) = model.results[index].clone() else {
                report.not_computed += 1;
                continue;
            };
            let document = model.documents[&formula.sheet];
            let style = model.style(document, formula.sheet, formula.row, formula.column);
            // A date entered by formula into a General cell shows as a date, as Excel formats it.
            let date_format = match (&value, &formula.expr) {
                (Value::Number(_), Some(expr)) if formula.entered && styles.format_id(style) == 0 => {
                    model.date_kind(expr, formula.sheet)
                }
                _ => None,
            };
            let date = date_format.map(|id| NumberFormat::parse(&date_code(id, &locale)));
            let format = date.as_ref().unwrap_or_else(|| formats.get(style).unwrap_or(&formats[0]));
            let (shown, computed) = show(&value, format, &locale, self.date1904);
            let current = document.field_at(formula.row as usize, formula.column as usize);
            let unchanged = matches!(current, Some(Field::Formula(cell))
                if cell.value == shown && cell.computed == computed && cell.date_format == date_format);
            if !unchanged {
                updates.entry(formula.sheet).or_default().push((formula.row as usize, formula.column as usize, shown, computed, date_format));
            }
        }
        drop(model);
        if self.computes_on_load {
            state.computed.extend(sheets.iter().map(|(index, _)| *index));
        }
        for (index, document) in sheets.iter_mut() {
            if let Some(results) = updates.remove(index) {
                // Rows given results count as edited from now on, and have not changed.
                let rows: BTreeSet<usize> = results.iter().map(|(row, ..)| *row).collect();
                document.set_formula_results(results);
                let entered = state.entered.entry(*index).or_default();
                for row in rows {
                    entered.entry(row as u32).or_insert_with(|| document.row_fingerprint(row));
                }
                report.sheets.push(*index);
            }
        }
        report.sheets.append(&mut shown_sheets);
        report.sheets.sort_unstable();
        report.sheets.dedup();
        Ok(report)
    }

    /// Values typed into rows changed since, shown as the number formats of their cells show them:
    /// `300` in a money cell as `300,00 ₽`, `2026-10-01` in a date cell as `01.10.2026`. A cell
    /// that shows what the file has keeps it. Returns the sheets whose cells changed.
    fn show_entered(
        &self,
        state: &mut CalcState,
        sheets: &mut [(usize, &mut Document)],
        touched: &HashMap<usize, BTreeSet<u32>>,
        formats: &[Arc<NumberFormat>],
        locale: &DisplayLocale,
    ) -> Vec<usize> {
        let mut changed = Vec::new();
        for (index, document) in sheets.iter_mut() {
            let rows: Vec<usize> = touched
                .get(index)
                .into_iter()
                .flatten()
                .map(|&row| row as usize)
                .filter(|&row| row < document.row_count() && document.row_edited(row))
                .collect();
            if rows.is_empty() {
                continue;
            }
            let raw = match state.raw.get(index) {
                Some(raw) => Arc::clone(raw),
                None => {
                    // Without the file's values, typed values show as typed.
                    let Ok(raw) = self.read_raw(*index, locale) else { continue };
                    let raw = Arc::new(raw);
                    state.raw.insert(*index, Arc::clone(&raw));
                    raw
                }
            };
            let mut shown = Vec::new();
            for row in rows {
                let origin = document.row_origin(row);
                for (column, field) in document.row_fields(row).iter().enumerate() {
                    let Field::Text(text) = field else { continue };
                    if text.is_empty() {
                        continue;
                    }
                    let original = origin.zip(document.column_origin(column)).and_then(|(row, column)| raw.get(row, column));
                    // A cell of an added row or column formats as its neighbour of the file.
                    let style = document.format_source(row, column).and_then(|(row, column)| raw.get(row, column)).map_or(0, |cell| cell.style);
                    let format = self.format_of(formats, style);
                    if let Some(cell) = original
                        && show(&raw_value(Some(cell), &raw), format, locale, self.date1904).0 == *text
                    {
                        continue;
                    }
                    let value = match super::save::typed(text, format, locale, self.date1904) {
                        super::save::Typed::Number(number) => Value::Number(number),
                        super::save::Typed::Bool(truth) => Value::Bool(truth),
                        super::save::Typed::Text(_) => continue,
                    };
                    let (formatted, _) = show(&value, format, locale, self.date1904);
                    if formatted != *text {
                        shown.push((row, column, formatted));
                    }
                }
            }
            if !shown.is_empty() {
                document.set_entered_shown(shown);
                changed.push(*index);
            }
        }
        changed
    }

    fn format_of<'a>(&self, formats: &'a [Arc<NumberFormat>], style: u32) -> &'a NumberFormat {
        formats.get(style as usize).unwrap_or(&formats[0])
    }

    /// What the file holds in the cells of a sheet, to keep exact values when rows move.
    pub(crate) fn file_values(&self, index: usize, locale: &DisplayLocale) -> Option<FileValues> {
        let Source::Xlsx { styles, .. } = &self.source else { return None };
        let mut state = self.calc.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let raw = match state.raw.get(&index) {
            Some(raw) => Arc::clone(raw),
            None => {
                let raw = Arc::new(self.read_raw(index, locale).ok()?);
                state.raw.insert(index, Arc::clone(&raw));
                raw
            }
        };
        Some(FileValues { raw, formats: styles.formats(locale), locale: locale.clone(), date1904: self.date1904 })
    }

    /// The values a sheet stores, with the style of each cell.
    fn read_raw(&self, index: usize, locale: &DisplayLocale) -> Result<RawSheet> {
        let Source::Xlsx { package, strings, styles } = &self.source else {
            return Ok(RawSheet::default());
        };
        let Some(part) = self.sheets.get(index).and_then(|sheet| sheet.part.as_deref()) else {
            return Ok(RawSheet::default());
        };
        let formats = styles.formats(locale);
        let context = super::sheet::Context { strings, formats: &formats, locale, date1904: self.date1904 };
        // A large sheet is read on several threads.
        if let Some((mut stream, size)) = package.stream(part)?
            && size >= super::sheet::PARALLEL_SIZE
            && let super::sheet::Parallel::Read(content) = super::sheet::read_sheet_parallel(&mut stream, size, part, &context, None, true)?
        {
            return Ok(content.and_then(|content| content.raw).unwrap_or_default());
        }
        let Some(text) = package.read_text(part)? else {
            return Ok(RawSheet::default());
        };
        let content = super::sheet::read_sheet(&mut super::xml::reader(&text), text.len() as u64, part, &context, None, true)?;
        Ok(content.and_then(|content| content.raw).unwrap_or_default())
    }
}

/// What the file holds in the cells of a sheet.
pub(crate) struct FileValues {
    raw: Arc<RawSheet>,
    formats: Vec<Arc<NumberFormat>>,
    locale: DisplayLocale,
    date1904: bool,
}

impl FileValues {
    /// The file's exact value of a cell that shows `shown` as the file has it, as text read
    /// back exactly: a plain number, TRUE or FALSE, or the text. `None` when it shows
    /// something else or holds an error.
    pub(crate) fn exact(&self, row: usize, column: usize, shown: &str) -> Option<String> {
        let cell = self.raw.get(row, column)?;
        let value = raw_value(Some(cell), &self.raw);
        let format = self.formats.get(cell.style as usize).unwrap_or(&self.formats[0]);
        if show(&value, format, &self.locale, self.date1904).0 != shown {
            return None;
        }
        match value {
            Value::Number(number) => Some(format!("{number}")),
            Value::Bool(truth) => Some((if truth { "TRUE" } else { "FALSE" }).to_owned()),
            Value::Text(text) => Some(text),
            Value::Error(_) | Value::Empty => None,
        }
    }
}

/// The cells formulas read, while computing them.
struct Model<'a> {
    workbook: &'a Workbook,
    names: &'a Names,
    tables: &'a [TableDef],
    documents: HashMap<usize, &'a Document>,
    raw: &'a mut HashMap<usize, Arc<RawSheet>>,
    formats: &'a [Arc<NumberFormat>],
    locale: &'a DisplayLocale,
    formulas: &'a [Formula],
    /// Formulas to compute, by sheet, row and column.
    positions: HashMap<(usize, u32, u32), usize>,
    results: Vec<Option<std::result::Result<Value, Unsupported>>>,
    /// Formulas being computed: reaching one again is a circular reference.
    busy: Vec<bool>,
    /// The formulas being computed, innermost last; those in a circle show 0, as in Excel.
    stack: Vec<usize>,
    in_circle: Vec<bool>,
    circular: usize,
    now: f64,
    failed: Option<crate::error::Error>,
}

impl Model<'_> {
    fn result(&mut self, index: usize) -> std::result::Result<Value, Unsupported> {
        if let Some(result) = &self.results[index] {
            return result.clone();
        }
        if self.busy[index] {
            self.circular += 1;
            // Every formula from this one to the innermost reads itself through the others.
            if let Some(start) = self.stack.iter().position(|&busy| busy == index) {
                for &member in &self.stack[start..] {
                    self.in_circle[member] = true;
                }
            }
            return Ok(Value::Number(0.0));
        }
        self.busy[index] = true;
        self.stack.push(index);
        let formula = &self.formulas[index];
        let (sheet, row, column, expr) = (formula.sheet, formula.row, formula.column, formula.expr.clone());
        let array = formula.array;
        let result = match expr {
            Some(expr) => Evaluator { cells: self, sheet, row, column, array, names: 0 }.evaluate(&expr),
            None => Err(Unsupported),
        };
        self.busy[index] = false;
        self.stack.pop();
        let result = if self.in_circle[index] { Ok(Value::Number(0.0)) } else { result };
        self.results[index] = Some(result.clone());
        result
    }

    fn raw(&mut self, sheet: usize) -> Arc<RawSheet> {
        if let Some(raw) = self.raw.get(&sheet) {
            return Arc::clone(raw);
        }
        let raw = match self.workbook.read_raw(sheet, self.locale) {
            Ok(raw) => Arc::new(raw),
            Err(error) => {
                self.failed.get_or_insert(error);
                Arc::new(RawSheet::default())
            }
        };
        self.raw.insert(sheet, Arc::clone(&raw));
        raw
    }

    /// The style of a cell of a sheet: the one the file gives it.
    fn style(&mut self, document: &Document, sheet: usize, row: u32, column: u32) -> usize {
        let Some((row, column)) = document.format_source(row as usize, column as usize) else { return 0 };
        self.raw(sheet).get(row, column).map_or(0, |cell| cell.style as usize)
    }

    /// Whether an expression gives a date, as Excel tells when it formats a General cell a
    /// formula is entered into: the date functions, a date cell, a date plus or minus days.
    /// The built-in format it then gets: 14, the short date, or 22 with the time for NOW.
    fn date_kind(&mut self, expr: &Expr, sheet: usize) -> Option<u32> {
        match expr {
            Expr::Call(name, _) => match name.trim_start_matches("_xlfn.").to_uppercase().as_str() {
                "TODAY" | "DATE" | "EDATE" | "EOMONTH" | "DATEVALUE" | "WORKDAY" => Some(14),
                "NOW" => Some(22),
                _ => None,
            },
            Expr::Unary(UnaryOp::Plus, operand) => self.date_kind(operand, sheet),
            Expr::Binary(op @ (BinaryOp::Add | BinaryOp::Subtract), left, right) => {
                match (self.date_kind(left, sheet), self.date_kind(right, sheet)) {
                    (Some(kind), None) => Some(kind),
                    (None, Some(kind)) if *op == BinaryOp::Add => Some(kind),
                    _ => None,
                }
            }
            Expr::Reference(reference) if reference.area.is_cell() => {
                let target = match &reference.sheet {
                    None => sheet,
                    Some(name) => self.sheet(name)?,
                };
                let (row, column) = (reference.area.top, reference.area.left);
                let style = match self.documents.get(&target).copied() {
                    Some(document) => self.style(document, target, row, column),
                    None => self.raw(target).get(row as usize, column as usize).map_or(0, |cell| cell.style as usize),
                };
                self.formats.get(style).is_some_and(|format| format.is_date()).then_some(14)
            }
            _ => None,
        }
    }

    fn format(&self, style: u32) -> &NumberFormat {
        self.formats.get(style as usize).unwrap_or(&self.formats[0])
    }
}

impl Cells for Model<'_> {
    fn sheet(&self, name: &str) -> Option<usize> {
        self.workbook.sheets.iter().position(|sheet| sheet.name.eq_ignore_ascii_case(name) || sheet.name.to_lowercase() == name.to_lowercase())
    }

    fn value(&mut self, sheet: usize, row: u32, column: u32) -> std::result::Result<Value, Unsupported> {
        if let Some(&index) = self.positions.get(&(sheet, row, column)) {
            return self.result(index);
        }
        let raw = self.raw(sheet);
        let Some(document) = self.documents.get(&sheet).copied() else {
            return Ok(raw_value(raw.get(row as usize, column as usize), &raw));
        };
        let field = document.field_at(row as usize, column as usize);
        if let Some(Field::Formula(cell)) = field {
            return Ok(computed_value(cell));
        }
        let original = document.row_origin(row as usize).and_then(|origin| raw.get(origin, column as usize));
        if !document.row_edited(row as usize) {
            return Ok(raw_value(original, &raw));
        }
        let text = match field {
            Some(Field::Text(text)) => text.as_str(),
            Some(Field::Entered(cell)) if cell.exact => {
                return Ok(match super::save::entered_value(cell, &NumberFormat::general(), self.locale, self.workbook.date1904) {
                    super::save::Typed::Number(number) => Value::Number(number),
                    super::save::Typed::Bool(truth) => Value::Bool(truth),
                    super::save::Typed::Text(text) => Value::Text(text.to_owned()),
                });
            }
            Some(Field::Entered(cell)) => cell.input.as_str(),
            _ => "",
        };
        if text.is_empty() {
            return Ok(Value::Empty);
        }
        // An edited row: cells that still show what the file has keep its exact values.
        let style = original.map_or(0, |cell| cell.style);
        let format = self.format(style);
        if let Some(cell) = original
            && show(&raw_value(Some(cell), &raw), format, self.locale, self.workbook.date1904).0 == text
        {
            return Ok(raw_value(Some(cell), &raw));
        }
        Ok(match super::save::typed(text, format, self.locale, self.workbook.date1904) {
            super::save::Typed::Number(number) => Value::Number(number),
            super::save::Typed::Bool(truth) => Value::Bool(truth),
            super::save::Typed::Text(text) => Value::Text(text.to_owned()),
        })
    }

    fn extent(&self, sheet: usize) -> (u32, u32) {
        match self.documents.get(&sheet) {
            Some(document) => (document.row_count().saturating_sub(1) as u32, document.column_count().saturating_sub(1) as u32),
            None => match self.raw.get(&sheet) {
                Some(raw) => (
                    raw.rows.len().saturating_sub(1) as u32,
                    raw.rows.iter().map(Vec::len).max().unwrap_or(1).saturating_sub(1) as u32,
                ),
                None => (0, 0),
            },
        }
    }

    fn locale(&self) -> &DisplayLocale {
        self.locale
    }

    fn date1904(&self) -> bool {
        self.workbook.date1904
    }

    fn now(&self) -> f64 {
        self.now
    }

    fn name(&self, name: &str, sheet: usize) -> Option<Arc<Expr>> {
        defined_name(self.names, name, sheet).cloned()
    }

    fn table(&self, sheet: usize, row: u32, column: u32, reference: &TableRef) -> Option<(usize, Area)> {
        table_cells(self.tables, Place { sheet, row, column }, reference)
    }
}

fn raw_value(cell: Option<&RawCell>, raw: &RawSheet) -> Value {
    let text = |index: u32| raw.texts.get(index as usize).cloned().unwrap_or_default();
    match cell.map_or(RawValue::Empty, |cell| cell.value) {
        RawValue::Empty => Value::Empty,
        RawValue::Number(number) => Value::Number(number),
        RawValue::Text(index) => Value::Text(text(index)),
        RawValue::Bool(truth) => Value::Bool(truth),
        RawValue::Error(index) => Value::Error(text(index)),
    }
}

fn computed_value(cell: &FormulaField) -> Value {
    match &cell.computed {
        Computed::Number(number) => Value::Number(*number),
        Computed::Text(text) => Value::Text(text.clone()),
        Computed::Bool(truth) => Value::Bool(*truth),
        Computed::Error(code) => Value::Error(code.clone()),
        Computed::Unknown if cell.value.is_empty() => Value::Empty,
        Computed::Unknown => Value::Text(cell.value.clone()),
    }
}

/// The code of a built-in date format as this Mac shows it: 14 the short date, 22 with the time.
pub(crate) fn date_code(id: u32, locale: &DisplayLocale) -> String {
    if id == 22 { format!("{} hh:mm", locale.short_date) } else { locale.short_date.clone() }
}

/// A result as a cell of this format shows it, and as it is kept.
fn show(value: &Value, format: &NumberFormat, locale: &DisplayLocale, date1904: bool) -> (String, Computed) {
    match value {
        Value::Number(number) => (format.format_number(*number, locale, date1904), Computed::Number(*number)),
        Value::Text(text) => (format.format_text(text), Computed::Text(text.clone())),
        Value::Bool(truth) => ((if *truth { &locale.true_text } else { &locale.false_text }).clone(), Computed::Bool(*truth)),
        Value::Error(code) => (code.clone(), Computed::Error(code.clone())),
        // A formula that reads an empty cell gives 0.
        Value::Empty => (format.format_number(0.0, locale, date1904), Computed::Number(0.0)),
    }
}

/// The cells an expression reads, with sheets named in it found among `names` (lower case).
/// Defined names parsed, by the sheet they belong to (`None` for the workbook) and name.
pub(crate) type Names = HashMap<(Option<usize>, String), Arc<Expr>>;

/// The expression of a defined name used on a sheet: the sheet's own name first, as Excel
/// finds it, then the workbook's.
fn defined_name<'a>(names: &'a Names, name: &str, sheet: usize) -> Option<&'a Arc<Expr>> {
    let name = name.to_lowercase();
    names.get(&(Some(sheet), name.clone())).or_else(|| names.get(&(None, name)))
}

/// Names refer to names at most this deep; deeper is taken as a loop.
const NAME_DEPTH: u8 = 8;

impl Workbook {
    /// The Excel tables where they are now: rows and columns of the sheets given may have
    /// been inserted, deleted or moved since the file was read.
    fn current_tables(&self, sheets: &[(usize, &mut Document)]) -> Vec<TableDef> {
        self.tables()
            .iter()
            .filter_map(|table| {
                let Some((_, document)) = sheets.iter().find(|(index, _)| *index == table.sheet) else {
                    return Some(table.clone());
                };
                table.current(document)
            })
            .collect()
    }

    /// The defined names whose formulas read, parsed.
    fn defined_names(&self) -> Names {
        self.names
            .iter()
            .filter(|name| !name.name.starts_with("_xlnm."))
            .filter_map(|name| Some(((name.sheet, name.name.clone()), Arc::new(parse::parse(&name.formula).ok()?))))
            .collect()
    }
}

/// Where a formula is.
#[derive(Clone, Copy)]
struct Place {
    sheet: usize,
    row: u32,
    column: u32,
}

/// The cells a structured reference points to, for a formula at `place`: the table by name,
/// or the one the formula is in, and its rows and columns as the reference says.
fn table_cells(tables: &[TableDef], place: Place, reference: &TableRef) -> Option<(usize, Area)> {
    let table = match &reference.table {
        Some(name) => tables.iter().find(|table| table.display == *name || table.name == *name)?,
        None => tables.iter().find(|table| {
            table.sheet == place.sheet
                && (table.top..=table.bottom).contains(&place.row)
                && (table.left..=table.right).contains(&place.column)
        })?,
    };
    let data = (table.top + table.headers, table.bottom.saturating_sub(table.totals));
    let (top, bottom) = match reference.rows {
        TableRows::Data => data,
        TableRows::All => (table.top, table.bottom),
        TableRows::Headers if table.headers > 0 => (table.top, table.top + table.headers - 1),
        TableRows::Totals if table.totals > 0 => (table.bottom + 1 - table.totals, table.bottom),
        TableRows::ThisRow if table.sheet == place.sheet && (data.0..=data.1).contains(&place.row) => (place.row, place.row),
        _ => return None,
    };
    let (left, right) = match &reference.columns {
        Some((first, last)) => {
            let index = |name: &String| table.columns.iter().position(|column| column == name).map(|index| table.left + index as u32);
            let (first, last) = (index(first)?, index(last)?);
            (first.min(last), first.max(last))
        }
        None => (table.left, table.right),
    };
    (top <= bottom).then_some((table.sheet, Area { top, left, bottom, right }))
}

fn references(expr: &Expr, place: Place, names: &[String], defined: &Names, tables: &[TableDef], depth: u8, out: &mut Vec<(usize, Area)>) {
    let sheet = place.sheet;
    let recurse = |expr: &Expr, out: &mut Vec<(usize, Area)>| references(expr, place, names, defined, tables, depth, out);
    match expr {
        Expr::Reference(reference) => {
            let target = match &reference.sheet {
                None => Some(sheet),
                Some(name) => names.iter().position(|known| *known == name.to_lowercase()),
            };
            if let Some(target) = target {
                out.push((target, reference.area));
            }
        }
        Expr::Unary(_, operand) => recurse(operand, out),
        Expr::Binary(_, left, right) => {
            recurse(left, out);
            recurse(right, out);
        }
        Expr::Call(_, arguments) => {
            for argument in arguments {
                recurse(argument, out);
            }
        }
        // A name reads what its formula reads.
        Expr::Name(name) if depth < NAME_DEPTH => {
            if let Some(expr) = defined_name(defined, name, sheet) {
                references(expr, place, names, defined, tables, depth + 1, out);
            }
        }
        Expr::Table(reference) => out.extend(table_cells(tables, place, reference)),
        _ => {}
    }
}

impl Document {
    /// The field of a cell, if the row has one there.
    pub(crate) fn field_at(&self, row: usize, column: usize) -> Option<&Field> {
        match self.records.get(row)?.get() {
            crate::document::RecordRef::Modified(modified) => modified.fields.get(column),
            crate::document::RecordRef::Original { .. } => None,
        }
    }

    /// Cells with a formula, by row and column.
    pub(crate) fn formula_cells(&self) -> impl Iterator<Item = (usize, usize, &FormulaField)> {
        self.records.iter().enumerate().flat_map(|(row, record)| {
            let fields: &[Field] = match record.get() {
                crate::document::RecordRef::Modified(modified) => &modified.fields,
                crate::document::RecordRef::Original { .. } => &[],
            };
            fields.iter().enumerate().filter_map(move |(column, field)| match field {
                Field::Formula(cell) => Some((row, column, &**cell)),
                _ => None,
            })
        })
    }
}

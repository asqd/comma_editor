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
use self::parse::{Area, Expr};
use super::numfmt::{DisplayLocale, NumberFormat};
use super::sheet::{RawCell, RawSheet, RawValue};
use super::{Source, Workbook};
use crate::document::{Computed, Document, Field, FormulaField};
use crate::error::Result;
use crate::formula::FormulaError;

/// A formula as a person typed it after `=`, written the way files keep it.
pub(crate) fn normalize(text: &str) -> std::result::Result<String, FormulaError> {
    parse::normalize(text)
}

/// What computing lasts across edits: values the file stores, and rows edited so far.
#[derive(Default)]
pub(crate) struct CalcState {
    raw: HashMap<usize, Arc<RawSheet>>,
    /// Rows edited since the workbook was opened, by sheet. They stay after an undo, so that
    /// formulas reading them are computed back.
    pub(crate) touched: HashMap<usize, BTreeSet<u32>>,
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
        for (index, document) in sheets.iter() {
            let touched = state.touched.entry(*index).or_default();
            touched.extend((0..document.row_count()).filter(|&row| document.row_edited(row)).map(|row| row as u32));
        }
        let has_formulas = sheets.iter().any(|(_, document)| document.formula_cells().next().is_some());
        if !has_formulas {
            return Ok(Recalculation::default());
        }

        // Every formula of the sheets, read once per text.
        let names: Vec<String> = self.sheets.iter().map(|sheet| sheet.name.to_lowercase()).collect();
        let mut parsed: HashMap<String, Option<Arc<Expr>>> = HashMap::new();
        let mut formulas = Vec::new();
        for (index, document) in sheets.iter() {
            for (row, column, cell) in document.formula_cells() {
                let expr = parsed.entry(cell.formula.clone()).or_insert_with(|| parse::parse(&cell.formula).ok().map(Arc::new)).clone();
                let mut reads = Vec::new();
                if let Some(expr) = &expr {
                    references(expr, *index, &names, &mut reads);
                }
                formulas.push(Formula { sheet: *index, row: row as u32, column: column as u32, expr, reads, entered: !cell.from_file });
            }
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
                    state.touched.get(sheet).is_some_and(|rows| rows.range(area.top..=area.bottom).next().is_some())
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
            return Ok(Recalculation::default());
        }

        let formats = styles.formats(&locale);
        let documents: HashMap<usize, &Document> = sheets.iter().map(|(index, document)| (*index, &**document)).collect();
        let positions: HashMap<(usize, u32, u32), usize> = formulas
            .iter()
            .enumerate()
            .filter(|(index, _)| marked[*index])
            .map(|(index, formula)| ((formula.sheet, formula.row, formula.column), index))
            .collect();
        let mut model = Model {
            workbook: self,
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
            return Err(error);
        }

        // Results shown in each cell's format, written where they changed.
        let mut report = Recalculation { circular: model.circular, ..Recalculation::default() };
        let mut updates: HashMap<usize, Vec<(usize, usize, String, Computed)>> = HashMap::new();
        for &index in &order {
            let formula = &formulas[index];
            let Some(Ok(value)) = model.results[index].clone() else {
                report.not_computed += 1;
                continue;
            };
            let document = model.documents[&formula.sheet];
            let style = model.style(document, formula.sheet, formula.row, formula.column);
            let format = formats.get(style).unwrap_or(&formats[0]);
            let (shown, computed) = show(&value, format, &locale, self.date1904);
            let current = document.field_at(formula.row as usize, formula.column as usize);
            let unchanged = matches!(current, Some(Field::Formula(cell)) if cell.value == shown && cell.computed == computed);
            if !unchanged {
                updates.entry(formula.sheet).or_default().push((formula.row as usize, formula.column as usize, shown, computed));
            }
        }
        drop(model);
        if self.computes_on_load {
            state.computed.extend(sheets.iter().map(|(index, _)| *index));
        }
        for (index, document) in sheets.iter_mut() {
            if let Some(results) = updates.remove(index) {
                document.set_formula_results(results);
                report.sheets.push(*index);
            }
        }
        report.sheets.sort_unstable();
        Ok(report)
    }

    /// The values a sheet stores, with the style of each cell.
    fn read_raw(&self, index: usize, locale: &DisplayLocale) -> Result<RawSheet> {
        let Source::Xlsx { package, strings, styles } = &self.source else {
            return Ok(RawSheet::default());
        };
        let Some(part) = self.sheets.get(index).and_then(|sheet| sheet.part.as_deref()) else {
            return Ok(RawSheet::default());
        };
        let Some(text) = package.read_text(part)? else {
            return Ok(RawSheet::default());
        };
        let formats = styles.formats(locale);
        let context = super::sheet::Context { strings, formats: &formats, locale, date1904: self.date1904 };
        let content = super::sheet::read_sheet(&mut super::xml::reader(&text), text.len() as u64, part, &context, None, true)?;
        Ok(content.and_then(|content| content.raw).unwrap_or_default())
    }
}

/// The cells formulas read, while computing them.
struct Model<'a> {
    workbook: &'a Workbook,
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
        let result = match expr {
            Some(expr) => Evaluator { cells: self, sheet, row, column }.evaluate(&expr),
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
        let Some(origin) = document.row_origin(row as usize) else { return 0 };
        self.raw(sheet).get(origin, column as usize).map_or(0, |cell| cell.style as usize)
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
fn references(expr: &Expr, sheet: usize, names: &[String], out: &mut Vec<(usize, Area)>) {
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
        Expr::Unary(_, operand) => references(operand, sheet, names, out),
        Expr::Binary(_, left, right) => {
            references(left, sheet, names, out);
            references(right, sheet, names, out);
        }
        Expr::Call(_, arguments) => {
            for argument in arguments {
                references(argument, sheet, names, out);
            }
        }
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

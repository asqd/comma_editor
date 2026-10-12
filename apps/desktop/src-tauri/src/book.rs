//! An Excel workbook open in a window (WorkbookDocument in the macOS app). Each sheet is a table
//! of its own, read from the file when first shown; one sheet at a time is the window's table.
//! Saving writes the edits of every sheet into the file opened, changing only what changed.
//! Excel 97–2003 workbooks open for viewing only.
//!
//! The sheets keep their own undo history, as documents of the core do; the workbook keeps the
//! order of the steps across sheets, as the macOS app's undo manager does, so that undo goes
//! back through every sheet in turn and shows the sheet it changes.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use comma_ffi::{CoreDocument, CoreError, CoreWorkbook, DisplayLocale, SheetInfo, SheetJob, SheetToSave, SheetVisibility};
use serde::Deserialize;
use serde_json::{Value, json};

/// How formulas are shown: as Excel shows them where numbers have a decimal comma (`;` between
/// arguments, `1,5`), and with Russian function names.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormulaStyle {
    pub comma: bool,
    pub russian: bool,
}

/// How the system shows numbers and dates and what time it is there, from the interface.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Locale {
    pub display: DisplayLocale,
    #[serde(default)]
    pub formulas: FormulaStyle,
    /// Seconds east of UTC, for TODAY and NOW.
    #[serde(default)]
    pub utc_offset: i64,
}

/// A sheet being read in the background.
pub struct Loading {
    pub index: usize,
    pub job: Arc<SheetJob>,
    /// Counts reads, so that a cancelled one ending late leaves the next one alone.
    pub number: u64,
}

pub struct Book {
    pub workbook: Arc<CoreWorkbook>,
    pub sheets: Vec<SheetInfo>,
    pub legacy: bool,
    pub locale: Locale,
    pub loaded: BTreeMap<usize, Arc<CoreDocument>>,
    /// The layout Excel gives each sheet read: widths, hidden columns, frozen panes.
    layouts: HashMap<usize, Value>,
    /// The sheet shown.
    pub index: usize,
    pub loading: Option<Loading>,
    loads: u64,
    /// CSV mode: the first row of every sheet names its columns.
    pub csv_mode: bool,
    /// After the last computation: formulas that read themselves, and ones the core does not
    /// compute, which Excel computes when it opens the file.
    pub circular: u64,
    pub not_computed: u64,
    /// Steps, each a list of the sheets whose own step it took, in the order they were taken.
    undo: Vec<Vec<usize>>,
    redo: Vec<Vec<usize>>,
    /// The last change of each sheet as the steps above know it, and as saved.
    seen: HashMap<usize, Option<u64>>,
    saved: HashMap<usize, Option<u64>>,
    /// Bumps when computing changed what the shown sheet shows, so that its rows are fetched again.
    pub revision: u64,
    /// Why reading a sheet or computing formulas failed, until the interface is told.
    pub problem: Option<Value>,
}

/// An Excel workbook by its name or by its first bytes: a ZIP package or an OLE file.
pub fn is_workbook(path: &str) -> bool {
    let extension = Path::new(path).extension().map(|extension| extension.to_string_lossy().to_lowercase());
    if matches!(extension.as_deref(), Some("xlsx" | "xlsm" | "xls")) {
        return true;
    }
    let mut head = [0u8; 4];
    std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok()
        && (head == [0x50, 0x4B, 0x03, 0x04] || head == [0xD0, 0xCF, 0x11, 0xE0])
}

/// Why a workbook cannot be shown, in words the interface translates.
pub const NO_TABLES: &str = "The workbook has no sheets with cells.";

impl Book {
    /// Opens a workbook at the sheet Excel opens at, reading the sheets the file asks to
    /// compute on opening too, and computes their formulas.
    pub fn open(path: &str, locale: Locale) -> Result<Self, Opening> {
        let workbook = CoreWorkbook::open(path.to_owned())?;
        let sheets = workbook.sheets();
        if !sheets.iter().any(|sheet| sheet.is_table) {
            return Err(Opening::NoTables);
        }
        let index = workbook.active_sheet() as usize;
        let mut book = Self {
            legacy: workbook.is_legacy_format(),
            workbook,
            sheets,
            locale,
            loaded: BTreeMap::new(),
            layouts: HashMap::new(),
            index,
            loading: None,
            loads: 0,
            csv_mode: false,
            circular: 0,
            not_computed: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            seen: HashMap::new(),
            saved: HashMap::new(),
            revision: 0,
            problem: None,
        };
        book.read_now(index)?;
        // A file written by a program asks to be computed on opening, as Excel does: its sheets
        // with formulas are read now, so formulas reading other sheets get results.
        for other in book.workbook.sheets_computed_on_open()? {
            book.read_now(other as usize)?;
        }
        // A formula that cannot be computed keeps its stored result: opening goes on.
        let _ = book.recalculate();
        Ok(book)
    }

    /// The sheet shown.
    pub fn shown(&self) -> Arc<CoreDocument> {
        self.loaded[&self.index].clone()
    }

    pub fn name(&self) -> &str {
        self.sheets.get(self.index).map_or("", |sheet| sheet.name.as_str())
    }

    pub fn layout(&self) -> Value {
        self.layouts.get(&self.index).cloned().unwrap_or(Value::Null)
    }

    /// Reads a sheet here and now, unless it was read before.
    fn read_now(&mut self, index: usize) -> Result<(), CoreError> {
        if self.loaded.contains_key(&index) {
            return Ok(());
        }
        let job = SheetJob::new();
        if let Some(sheet) = self.workbook.read_sheet(index as u64, self.locale.display.clone(), job)? {
            self.adopt(index, sheet.document, serde_json::to_value(sheet.layout).unwrap_or(Value::Null));
        }
        Ok(())
    }

    /// Keeps a sheet just read, shown in the workbook's mode.
    pub fn adopt(&mut self, index: usize, document: Arc<CoreDocument>, layout: Value) {
        document.set_has_header(self.csv_mode);
        let id = document.last_change_id();
        self.seen.insert(index, id);
        self.saved.insert(index, id);
        self.loaded.insert(index, document);
        self.layouts.insert(index, layout);
    }

    /// Starts reading a sheet; the caller reads it off the lock with the job returned.
    pub fn begin_loading(&mut self, index: usize) -> Option<(Arc<SheetJob>, u64)> {
        if let Some(loading) = self.loading.take() {
            loading.job.cancel();
        }
        if self.loaded.contains_key(&index) {
            return None;
        }
        self.loads += 1;
        let job = SheetJob::new();
        self.loading = Some(Loading { index, job: job.clone(), number: self.loads });
        Some((job, self.loads))
    }

    pub fn cancel_loading(&mut self) {
        if let Some(loading) = self.loading.take() {
            loading.job.cancel();
        }
    }

    /// Shows a sheet read before; says whether it was.
    pub fn show(&mut self, index: usize) -> bool {
        if !self.loaded.contains_key(&index) {
            return false;
        }
        self.cancel_loading();
        self.index = index;
        // A sheet read just now may have formulas that read cells edited on other sheets.
        self.compute();
        true
    }

    /// The sheet before or after the shown one that can be shown: a visible table.
    pub fn neighbour(&self, step: isize) -> Option<usize> {
        let mut index = self.index as isize + step;
        while index >= 0 && (index as usize) < self.sheets.len() {
            let sheet = &self.sheets[index as usize];
            if sheet.is_table && matches!(sheet.visibility, SheetVisibility::Visible) {
                return Some(index as usize);
            }
            index += step;
        }
        None
    }

    pub fn to_save(&self) -> Vec<SheetToSave> {
        self.loaded.iter().map(|(index, document)| SheetToSave { index: *index as u64, document: document.clone() }).collect()
    }

    /// Now as Excel counts days: from 1900, in local time.
    fn now(&self) -> f64 {
        let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |time| time.as_secs_f64());
        (seconds + self.locale.utc_offset as f64) / 86_400.0 + 25_569.0
    }

    /// Computes what depends on changed cells; says whether the shown sheet changed.
    pub fn recalculate(&mut self) -> Result<bool, CoreError> {
        if self.legacy {
            return Ok(false);
        }
        let report = self.workbook.recalculate(self.to_save(), self.now())?;
        self.circular = report.circular;
        self.not_computed = report.not_computed;
        let shown = report.sheets.contains(&(self.index as u64));
        if shown {
            self.revision += 1;
        }
        Ok(shown)
    }

    /// Computes formulas, keeping a failure to tell.
    fn compute(&mut self) {
        if let Err(error) = self.recalculate() {
            self.problem = serde_json::to_value(error).ok();
        }
    }

    /// Takes in changes made to sheets since the last look, each a step of its own, and
    /// computes what depends on them.
    pub fn settle(&mut self) {
        let mut changed = Vec::new();
        for (index, document) in &self.loaded {
            let id = document.last_change_id();
            if self.seen.get(index) != Some(&id) {
                self.seen.insert(*index, id);
                changed.push(*index);
            }
        }
        if changed.is_empty() {
            return;
        }
        // The shown sheet's step first, so that undoing ends on it.
        changed.sort_by_key(|index| *index != self.index);
        self.undo.push(changed);
        self.redo.clear();
        self.compute();
    }

    /// Runs a change that moves references in every sheet: each sheet is read first, so no
    /// formula is missed; the sheets changed take one undo step together. `false` when nothing
    /// changed.
    pub fn change(&mut self, change: impl FnOnce(&CoreWorkbook, Vec<SheetToSave>, u64) -> Result<Vec<u64>, CoreError>) -> Result<bool, CoreError> {
        if self.legacy {
            return Err(CoreError::Workbook { problem: comma_ffi::WorkbookProblem::ReadOnly });
        }
        self.settle();
        for index in 0..self.sheets.len() {
            if self.sheets[index].is_table {
                self.read_now(index)?;
            }
        }
        let changed = change(&self.workbook, self.to_save(), self.index as u64)?;
        if changed.is_empty() {
            return Ok(false);
        }
        let shown = self.index as u64;
        let mut order = vec![shown];
        for sheet in &changed {
            if !order.contains(sheet) {
                order.push(*sheet);
            }
        }
        let step: Vec<usize> = order
            .iter()
            .flat_map(|sheet| std::iter::repeat_n(*sheet as usize, changed.iter().filter(|changed| *changed == sheet).count()))
            .collect();
        for index in &step {
            self.seen.insert(*index, self.loaded[index].last_change_id());
        }
        self.undo.push(step);
        self.redo.clear();
        self.compute();
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last step, last sheet first, and shows the sheet it was taken on.
    pub fn undo(&mut self) -> bool {
        self.settle();
        let Some(step) = self.undo.pop() else { return false };
        for index in step.iter().rev() {
            if let Some(document) = self.loaded.get(index) {
                document.undo();
                self.seen.insert(*index, document.last_change_id());
            }
        }
        self.cancel_loading();
        self.index = step[0];
        self.redo.push(step);
        self.compute();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.settle();
        let Some(step) = self.redo.pop() else { return false };
        for index in &step {
            if let Some(document) = self.loaded.get(index) {
                document.redo();
                self.seen.insert(*index, document.last_change_id());
            }
        }
        self.cancel_loading();
        self.index = step[0];
        self.undo.push(step);
        self.compute();
        true
    }

    pub fn dirty(&self) -> bool {
        self.loaded.iter().any(|(index, document)| self.saved.get(index) != Some(&document.last_change_id()))
    }

    /// The workbook with the edits of every sheet read, written beside `path` and moved over
    /// it, so that a failed save leaves the file as it was.
    pub fn save(&mut self, path: &str) -> Result<(), crate::session::Failure> {
        if self.legacy {
            return Err(CoreError::Workbook { problem: comma_ffi::WorkbookProblem::ReadOnly }.into());
        }
        let bytes = self.workbook.bytes(self.to_save())?;
        let target = Path::new(path);
        let file = target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let temporary = target.with_file_name(format!(".{file}.{}.saving", std::process::id()));
        let written = std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, target));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(CoreError::Io { message: error.to_string(), code: error.raw_os_error() }.into());
        }
        for (index, document) in &self.loaded {
            self.saved.insert(*index, document.last_change_id());
        }
        Ok(())
    }

    /// CSV mode on or off for every sheet.
    pub fn set_csv_mode(&mut self, on: bool) {
        self.csv_mode = on;
        for document in self.loaded.values() {
            document.set_has_header(on);
        }
    }

    /// What the interface shows of the workbook.
    pub fn status(&mut self) -> Value {
        let loading = self.loading.as_ref().map(|loading| json!({ "index": loading.index, "fraction": loading.job.fraction() }));
        let info = self.loaded.get(&self.index).map(|document| document.info());
        json!({
            "sheets": self.sheets,
            "index": self.index,
            "loading": loading,
            "legacy": self.legacy,
            "csvMode": self.csv_mode,
            "circular": self.circular,
            "notComputed": self.not_computed,
            "rowsFixed": info.as_ref().is_some_and(|info| info.sheet_rows_fixed),
            "structureLocked": info.as_ref().is_some_and(|info| info.sheet_structure_locked),
            "problem": self.problem.take(),
        })
    }

    /// A formula as the interface shows it, with its `=`.
    pub fn shown_formula(&self, formula: String) -> String {
        let style = self.locale.formulas;
        let formula = if style.comma { comma_ffi::localize_formula(formula, style.russian) } else { formula };
        format!("={formula}")
    }
}

/// Why a workbook did not open.
pub enum Opening {
    Core(CoreError),
    NoTables,
}

impl From<CoreError> for Opening {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

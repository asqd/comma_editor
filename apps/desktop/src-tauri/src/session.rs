//! The documents of the windows and the commands the interface sends them.
//!
//! Every command goes through `Sessions::dispatch`: the Tauri app calls it from its one `core`
//! command, and the development bridge (`src/bin/bridge.rs`) from HTTP, so that the interface
//! behaves the same in the app and in a browser. Rows are visible rows unless a name says
//! otherwise: data rows as the current sort and filters show them, without the header.
//!
//! Commands block; callers run them off the main thread. Work that can take seconds on a large
//! file (indexing, sorting and filtering, replacing everywhere) runs in a thread of its own,
//! and the interface follows it through `status`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use comma_ffi::{
    CellPosition, CellRange, CoreDocument, CoreError, EditJob, EditRequest, ExportFormat, FilePreview, LineEnding, RangeStats,
    SaveFormat, SearchQuery, TextEncoding, ViewJob, ViewSpec,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Bytes indexed per background step, as `CSVDocument.indexingStep` in the macOS app.
const INDEXING_STEP: u64 = 64 << 20;
/// Tables up to this many rows are sorted, filtered and replaced in at once, as in the macOS
/// app; larger ones in a background job.
const JOB_ROWS: u64 = 200_000;

/// Why a command failed: an error of the core, which the interface words itself, or a message.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Failure {
    Core(CoreError),
    Message { message: String },
}

impl Failure {
    pub fn message(message: impl ToString) -> Self {
        Self::Message { message: message.to_string() }
    }
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

type Outcome<T> = Result<T, Failure>;

/// What the window shows of its document, sent after every change.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    path: Option<String>,
    /// Data rows shown by the sort and filters.
    rows: u64,
    /// All data rows.
    data_rows: u64,
    columns: u64,
    /// Column titles from the header row; empty when the first row is data.
    header: Vec<String>,
    has_header: bool,
    encoding: String,
    has_bom: bool,
    delimiter: String,
    line_ending: LineEnding,
    detection_confident: bool,
    /// Changed since it was opened or saved.
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
    /// The change the next undo reverts: the rows to show differ whenever it does.
    change: Option<u64>,
    view: ViewSpec,
    /// Share of a large file read so far, while it is read.
    indexing: Option<f64>,
    /// Work running in the background.
    job: Option<JobStatus>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    /// `view` for sorting and filtering, `edit` for a change to many rows.
    kind: &'static str,
    /// Share done, for jobs that know it.
    fraction: Option<f64>,
}

/// Work in the background, and how it ended.
enum Job {
    View(Arc<ViewJob>),
    Edit(Arc<EditJob>),
}

struct Session {
    document: Arc<CoreDocument>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    path: Option<String>,
    /// `last_change_id` when the document was opened or saved.
    saved: Option<u64>,
    indexing: Option<f64>,
    job: Option<Job>,
    /// Restored from a draft: changed, though nothing was changed in this session.
    restored: bool,
    /// Counts jobs, so that a cancelled job ending late leaves the next one alone.
    job_number: u64,
    /// How the last job ended, until the interface takes it with `job_result`.
    finished: Option<Outcome<Value>>,
}

impl Session {
    fn new(document: Arc<CoreDocument>, path: Option<String>) -> Arc<Self> {
        let saved = document.last_change_id();
        Arc::new(Self { document, state: Mutex::new(State { path, saved, ..State::default() }) })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn status(&self) -> Status {
        let document = &self.document;
        let info = document.info();
        let header = if info.has_header { document.row(0).unwrap_or_default() } else { Vec::new() };
        let change = document.last_change_id();
        let state = self.state();
        Status {
            path: state.path.clone(),
            rows: info.visible_row_count,
            data_rows: info.row_count.saturating_sub(u64::from(info.has_header)),
            columns: info.column_count,
            header,
            has_header: info.has_header,
            encoding: info.encoding,
            has_bom: info.has_bom,
            delimiter: info.delimiter,
            line_ending: info.line_ending,
            detection_confident: info.detection_confident,
            dirty: state.restored || change != state.saved,
            can_undo: document.can_undo(),
            can_redo: document.can_redo(),
            change,
            view: document.view_spec(),
            indexing: state.indexing,
            job: state.job.as_ref().map(|job| match job {
                Job::View(_) => JobStatus { kind: "view", fraction: None },
                Job::Edit(job) => JobStatus { kind: "edit", fraction: Some(job.fraction()) },
            }),
        }
    }

    /// Indexes the rest of a large file in a thread of its own.
    fn index(self: &Arc<Self>) {
        if !self.document.is_indexing() {
            return;
        }
        self.state().indexing = Some(0.0);
        let session = self.clone();
        std::thread::spawn(move || {
            loop {
                let step = session.document.continue_indexing(INDEXING_STEP);
                let fraction = step.indexed_bytes as f64 / step.total_bytes.max(1) as f64;
                session.state().indexing = (!step.done).then_some(fraction);
                if step.done {
                    break;
                }
            }
        });
    }

    /// Runs `job` in a thread of its own. A new view replaces one still being computed, as a
    /// newer sort or filter makes the older one moot; other work waits for the running job.
    fn start(self: &Arc<Self>, job: Job) -> Outcome<()> {
        let mut state = self.state();
        match (&state.job, &job) {
            (None, _) => {}
            (Some(Job::View(running)), Job::View(_)) => running.cancel(),
            (Some(_), _) => return Err(Failure::Core(CoreError::Editing)),
        }
        state.job_number += 1;
        state.finished = None;
        let number = state.job_number;
        let session = self.clone();
        match &job {
            Job::View(view) => {
                let view = view.clone();
                std::thread::spawn(move || {
                    let shown = view.run() && session.document.finish_view(view);
                    session.finish(number, Ok(json!({ "shown": shown })));
                });
            }
            Job::Edit(edit) => {
                let edit = edit.clone();
                std::thread::spawn(move || {
                    edit.run();
                    let outcome = session.document.finish_edit(edit).map(|result| json!(result)).map_err(Failure::from);
                    session.finish(number, outcome);
                });
            }
        }
        state.job = Some(job);
        Ok(())
    }

    fn finish(&self, number: u64, outcome: Outcome<Value>) {
        let mut state = self.state();
        if state.job_number == number {
            state.job = None;
            state.finished = Some(outcome);
        }
    }
}

/// The open documents, by window.
#[derive(Default)]
pub struct Sessions {
    windows: Mutex<HashMap<String, Arc<Session>>>,
}

fn same(range: &CellRange) -> CellRange {
    CellRange { top: range.top, left: range.left, bottom: range.bottom, right: range.right }
}

fn args<T: DeserializeOwned>(args: Value) -> Outcome<T> {
    serde_json::from_value(args).map_err(Failure::message)
}

fn reply(value: impl Serialize) -> Outcome<Value> {
    serde_json::to_value(value).map_err(Failure::message)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Open {
    path: String,
    encoding: Option<TextEncoding>,
    delimiter: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preview {
    path: String,
    encoding: Option<TextEncoding>,
    delimiter: Option<String>,
    rows: u64,
}

#[derive(Deserialize)]
struct FilePath {
    path: String,
}

#[derive(Deserialize)]
struct Span {
    start: u64,
    count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetCell {
    row: u64,
    column: u64,
    value: String,
}

#[derive(Deserialize)]
struct Rename {
    column: u64,
    name: String,
}

#[derive(Deserialize)]
struct At {
    at: u64,
}

#[derive(Deserialize)]
struct InsertRows {
    at: u64,
    count: u64,
}

#[derive(Deserialize)]
struct RowRange {
    top: u64,
    bottom: u64,
}

#[derive(Deserialize)]
struct Range {
    range: CellRange,
}

#[derive(Deserialize)]
struct Fill {
    range: CellRange,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paste {
    /// The selection: a single value fills it, more are written from its top left.
    range: CellRange,
    text: String,
}

#[derive(Deserialize)]
struct Find {
    query: SearchQuery,
    from: Option<CellPosition>,
    forward: bool,
}

#[derive(Deserialize)]
struct Query {
    query: SearchQuery,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplaceInCell {
    row: u64,
    column: u64,
    query: SearchQuery,
    replacement: String,
}

#[derive(Deserialize)]
struct ReplaceAll {
    query: SearchQuery,
    replacement: String,
}

#[derive(Deserialize)]
struct ExportFile {
    path: String,
    format: ExportFormat,
    name: String,
}

#[derive(Deserialize)]
struct ExportText {
    format: ExportFormat,
    range: Option<CellRange>,
    name: String,
}

#[derive(Deserialize)]
struct Edit {
    request: EditRequest,
}

/// Makes a change to many rows: at once in a table of up to `JOB_ROWS` rows, as the macOS app
/// does, and in a background job in a larger one. `result` is `null` while the job runs.
fn edit(session: &Arc<Session>, request: EditRequest) -> Outcome<Value> {
    let document = &session.document;
    if document.row_count() < JOB_ROWS {
        let result = document.apply_edit(request)?;
        return Ok(json!({ "status": session.status(), "result": result }));
    }
    session.start(Job::Edit(document.begin_edit(request)?))?;
    Ok(json!({ "status": session.status(), "result": null }))
}

#[derive(Deserialize)]
struct SetView {
    view: ViewSpec,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HasHeader {
    has_header: bool,
}

#[derive(Deserialize)]
struct SaveAs {
    path: String,
    format: SaveFormat,
}

#[derive(Deserialize)]
struct Row {
    row: u64,
}

impl Sessions {
    fn session(&self, window: &str) -> Outcome<Arc<Session>> {
        let windows = self.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        windows.get(window).cloned().ok_or_else(|| Failure::message("no document in this window"))
    }

    fn set(&self, window: &str, session: Option<Arc<Session>>) {
        let mut windows = self.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match session {
            Some(session) => windows.insert(window.to_owned(), session),
            None => windows.remove(window),
        };
    }

    /// The window showing the file at `path`, if one does.
    pub fn window_for(&self, path: &str) -> Option<String> {
        let same = |a: &str| match (std::fs::canonicalize(a), std::fs::canonicalize(path)) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == path,
        };
        let windows = self.windows.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        windows
            .iter()
            .find(|(_, session)| session.state().path.as_deref().is_some_and(same))
            .map(|(window, _)| window.clone())
    }

    /// Writes the window's document to `file` in its own format, when it has changes and is
    /// small enough to copy every few seconds; returns the file it belongs to and the format, or
    /// `None` when there is nothing to keep.
    pub fn write_draft(&self, window: &str, file: &Path, largest: u64) -> Outcome<Option<(Option<String>, SaveFormat)>> {
        let session = self.session(window)?;
        let status = session.status();
        if !status.dirty || status.indexing.is_some() || status.job.is_some() {
            return Ok(None);
        }
        if let Some(path) = &status.path
            && std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > largest)
        {
            return Ok(None);
        }
        let format = session.document.save_format();
        session.document.save_with(file.to_string_lossy().into_owned(), format.clone())?;
        Ok(Some((status.path, format)))
    }

    /// Opens a draft in the window as the document of `path`, changed since it was saved, and
    /// saving in `format` as it did.
    pub fn restore(&self, window: &str, draft: &Path, path: Option<String>, format: SaveFormat) -> Outcome<Value> {
        let delimiter = Some(format.delimiter.clone());
        let document = CoreDocument::open_with(draft.to_string_lossy().into_owned(), Some(format.encoding), delimiter)?;
        document.set_save_format(format)?;
        let session = Session::new(document, path);
        session.state().restored = true;
        session.index();
        self.set(window, Some(session.clone()));
        reply(session.status())
    }

    /// Forgets the document of a window that closed.
    pub fn close(&self, window: &str) {
        self.set(window, None);
    }

    /// Runs `command` with `args` on the document of `window`. Commands that change the
    /// document return its new `Status`.
    pub fn dispatch(&self, window: &str, command: &str, args_: Value) -> Outcome<Value> {
        // Commands that make the window's document.
        match command {
            "open" => {
                let Open { path, encoding, delimiter } = args(args_)?;
                let document = CoreDocument::open_with(path.clone(), encoding, delimiter)?;
                let session = Session::new(document, Some(path));
                session.index();
                self.set(window, Some(session.clone()));
                return reply(session.status());
            }
            "new" => {
                let session = Session::new(CoreDocument::new_empty(), None);
                self.set(window, Some(session.clone()));
                return reply(session.status());
            }
            "close" => {
                self.set(window, None);
                return Ok(Value::Null);
            }
            "window_for_path" => {
                let FilePath { path } = args(args_)?;
                return reply(self.window_for(&path));
            }
            "preview" => {
                let Preview { path, encoding, delimiter, rows } = args(args_)?;
                let preview: FilePreview = comma_ffi::preview_file(path, encoding, delimiter, rows)?;
                return reply(preview);
            }
            _ => {}
        }

        let session = self.session(window)?;
        let document = &session.document;
        let status = |session: &Session| reply(session.status());
        match command {
            "status" => status(&session),
            "rows" => {
                let Span { start, count } = args(args_)?;
                reply(document.visible_rows(start, count))
            }
            "record" => {
                // The row of the file a visible row shows, to keep it selected across views.
                let Row { row } = args(args_)?;
                reply(document.visible_record(row))
            }
            "visible_index" => {
                let Row { row } = args(args_)?;
                reply(document.visible_index(row))
            }

            // Cells, rows and columns.
            "set_cell" => {
                let SetCell { row, column, value } = args(args_)?;
                let record = document.visible_record(row).ok_or_else(|| Failure::message("no such row"))?;
                document.set_cell(record, column, value)?;
                status(&session)
            }
            "rename_column" => {
                let Rename { column, name } = args(args_)?;
                document.set_cell(0, column, name)?;
                status(&session)
            }
            "insert_rows" => {
                let InsertRows { at, count } = args(args_)?;
                document.insert_rows(document.insertion_row(at), count)?;
                status(&session)
            }
            "delete_rows" => {
                // Visible rows `top..=bottom`: a range, as a whole table may be selected.
                let RowRange { top, bottom } = args(args_)?;
                let records = (top..=bottom).filter_map(|row| document.visible_record(row)).collect();
                document.delete_rows(records)?;
                status(&session)
            }
            "insert_column" => {
                let At { at } = args(args_)?;
                document.insert_column(at)?;
                status(&session)
            }
            "delete_column" => {
                let At { at } = args(args_)?;
                document.delete_column(at)?;
                status(&session)
            }
            "set_has_header" => {
                let HasHeader { has_header } = args(args_)?;
                document.set_has_header(has_header);
                status(&session)
            }
            "undo" => {
                document.undo();
                status(&session)
            }
            "redo" => {
                document.redo();
                status(&session)
            }

            // Ranges and the clipboard.
            "copy" => {
                let Range { range } = args(args_)?;
                reply(document.copy_range(range)?)
            }
            "fill" => {
                let Fill { range, value } = args(args_)?;
                document.fill_range(range, value)?;
                status(&session)
            }
            "fill_down" => {
                let Range { range } = args(args_)?;
                document.fill_down(range)?;
                status(&session)
            }
            "paste" => {
                let Paste { range, text } = args(args_)?;
                let grid = comma_ffi::parse_pasted_text(text);
                let single = grid.len() == 1 && grid[0].len() == 1;
                let written = if single && (range.bottom > range.top || range.right > range.left) {
                    document.fill_range(same(&range), grid[0][0].clone())?;
                    Some(range)
                } else {
                    document.paste(range.top, range.left, grid)?
                };
                Ok(json!({ "status": session.status(), "range": written }))
            }
            "range_stats" => {
                let Range { range } = args(args_)?;
                let stats: RangeStats = document.range_stats(range)?;
                reply(stats)
            }

            // Finding and replacing.
            "find" => {
                let Find { query, from, forward } = args(args_)?;
                reply(document.find(query, from, forward)?)
            }
            "count_matches" => {
                let Query { query } = args(args_)?;
                reply(document.count_matches(query)?)
            }
            "replace_in_cell" => {
                let ReplaceInCell { row, column, query, replacement } = args(args_)?;
                let replaced = document.replace_in_cell(row, column, query, replacement)?;
                Ok(json!({ "status": session.status(), "replaced": replaced }))
            }
            "replace_all" => {
                let ReplaceAll { query, replacement } = args(args_)?;
                edit(&session, EditRequest::ReplaceAll { query, replacement })
            }
            "edit" => {
                // A change to many rows: text, empty cells, splitting, merging, duplicates.
                let Edit { request } = args(args_)?;
                edit(&session, request)
            }

            // Sorting and filtering.
            "set_view" => {
                let SetView { view } = args(args_)?;
                if document.info().row_count <= JOB_ROWS {
                    document.set_view(view)?;
                } else {
                    session.start(Job::View(document.begin_view(view)?))?;
                }
                status(&session)
            }
            "apply_sort" => {
                document.apply_sort()?;
                status(&session)
            }
            "cancel_job" => {
                match &session.state().job {
                    Some(Job::View(job)) => job.cancel(),
                    Some(Job::Edit(job)) => job.cancel(),
                    None => {}
                }
                status(&session)
            }
            "job_result" => {
                // How the last background job ended, once.
                let finished = session.state().finished.take();
                match finished {
                    None => Ok(Value::Null),
                    Some(Ok(value)) => Ok(json!({ "ok": value })),
                    Some(Err(failure)) => Ok(json!({ "error": failure })),
                }
            }

            // Saving.
            "save_format" => reply(document.save_format()),
            "save" => {
                let path = session.state().path.clone().ok_or_else(|| Failure::message("the document has no file yet"))?;
                document.save(path)?;
                let mut state = session.state();
                state.saved = document.last_change_id();
                state.restored = false;
                drop(state);
                status(&session)
            }
            "save_as" => {
                // Saves in `format` from now on; the previous format stays if saving fails.
                let SaveAs { path, format } = args(args_)?;
                let previous = document.save_format();
                document.set_save_format(format)?;
                if let Err(error) = document.save(path.clone()) {
                    let _ = document.set_save_format(previous);
                    return Err(error.into());
                }
                let mut state = session.state();
                state.path = Some(path);
                state.saved = document.last_change_id();
                state.restored = false;
                drop(state);
                status(&session)
            }
            "save_copy" => {
                // A copy in another format; the document keeps its file and format.
                let SaveAs { path, format } = args(args_)?;
                document.save_with(path, format)?;
                Ok(Value::Null)
            }
            "export_file" => {
                // The shown rows in the shown order, written beside the target and then moved
                // over it, so that a failed export leaves no half file.
                let ExportFile { path, format, name } = args(args_)?;
                let target = std::path::PathBuf::from(&path);
                let file = target.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                let temporary = target.with_file_name(format!(".{file}.{}.exporting", std::process::id()));
                let written = document.export_to_file(temporary.to_string_lossy().into_owned(), format, name);
                let moved = written.map_err(Failure::from).and_then(|()| std::fs::rename(&temporary, &target).map_err(Failure::message));
                if moved.is_err() {
                    let _ = std::fs::remove_file(&temporary);
                }
                moved.map(|()| Value::Null)
            }
            "export_text" => {
                // Cells as Markdown, JSON, HTML or SQL for the clipboard: a fragment, without the
                // page around HTML or the table SQL creates.
                let ExportText { format, range, name } = args(args_)?;
                let bytes = document.export(format, range, name, true)?;
                reply(String::from_utf8_lossy(&bytes))
            }
            _ => Err(Failure::message(format!("unknown command {command}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy of a sample in a folder of its own, removed by the caller.
    fn sample(name: &str) -> (std::path::PathBuf, String) {
        let folder = std::env::temp_dir().join(format!("comma-session-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("people.csv");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../samples/russian-windows-1251.csv"), &path).unwrap();
        (folder, path.to_string_lossy().into_owned())
    }

    fn status(sessions: &Sessions, window: &str) -> Value {
        sessions.dispatch(window, "status", Value::Null).unwrap()
    }

    #[test]
    fn edits_are_dirty_until_saved_and_after_undo_they_are_not() {
        let (folder, path) = sample("dirty");
        let sessions = Sessions::default();
        sessions.dispatch("main", "open", json!({ "path": path })).unwrap();
        assert_eq!(status(&sessions, "main")["dirty"], false);
        sessions.dispatch("main", "set_cell", json!({ "row": 0, "column": 1, "value": "Тверь" })).unwrap();
        assert_eq!(status(&sessions, "main")["dirty"], true);
        sessions.dispatch("main", "undo", Value::Null).unwrap();
        assert_eq!(status(&sessions, "main")["dirty"], false);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_restored_draft_saves_as_the_document_would_have() {
        let (folder, path) = sample("draft");
        let sessions = Sessions::default();
        sessions.dispatch("main", "open", json!({ "path": path })).unwrap();
        sessions.dispatch("main", "set_cell", json!({ "row": 1, "column": 3, "value": "новый комментарий" })).unwrap();

        // What saving the edit straight away writes.
        let expected = folder.join("expected.csv");
        let format = sessions.dispatch("main", "save_format", Value::Null).unwrap();
        sessions.dispatch("main", "save_copy", json!({ "path": expected, "format": format })).unwrap();

        let draft = folder.join("main.draft");
        let (draft_path, draft_format) = sessions.write_draft("main", &draft, u64::MAX).unwrap().expect("a draft of the changes");
        assert_eq!(draft_path.as_deref(), Some(path.as_str()));

        // The app crashed; the next session restores the draft in a window of its own.
        let next = Sessions::default();
        let restored = next.restore("window-2", &draft, draft_path, draft_format).unwrap();
        assert_eq!(restored["dirty"], true);
        assert_eq!(restored["path"], path.as_str());
        assert_eq!(restored["encoding"], "Windows-1251");
        next.dispatch("window-2", "save", Value::Null).unwrap();
        assert_eq!(status(&next, "window-2")["dirty"], false);
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&expected).unwrap());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_unchanged_document_has_no_draft() {
        let (folder, path) = sample("clean");
        let sessions = Sessions::default();
        sessions.dispatch("main", "open", json!({ "path": path })).unwrap();
        assert!(sessions.write_draft("main", &folder.join("main.draft"), u64::MAX).unwrap().is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_window_showing_a_file_is_found_by_its_path() {
        let (folder, path) = sample("window");
        let sessions = Sessions::default();
        sessions.dispatch("document-3", "open", json!({ "path": path })).unwrap();
        let relative = folder.join(".").join("people.csv").to_string_lossy().into_owned();
        assert_eq!(sessions.window_for(&relative).as_deref(), Some("document-3"));
        assert_eq!(sessions.window_for("/nowhere.csv"), None);
        std::fs::remove_dir_all(folder).unwrap();
    }
}

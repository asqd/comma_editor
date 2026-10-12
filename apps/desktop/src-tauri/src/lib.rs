//! The Tauri shell of CommaEditor: commands over `comma-ffi`, one document per window.
//!
//! Commands are `async` so that they run off the main thread; reading rows takes the document's
//! lock only briefly, so the table scrolls while a large file is still being indexed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use comma_ffi::CoreDocument;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};

/// Bytes indexed per background step, as `CSVDocument.indexingStep` in the macOS app.
const INDEXING_STEP: u64 = 64 << 20;

/// The open document of each window, by window label.
#[derive(Default)]
struct Documents(Mutex<HashMap<String, Arc<CoreDocument>>>);

impl Documents {
    fn get(&self, window: &WebviewWindow) -> Result<Arc<CoreDocument>, String> {
        let documents = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        documents.get(window.label()).cloned().ok_or_else(|| "no file is open in this window".to_owned())
    }

    fn set(&self, window: &str, document: Option<Arc<CoreDocument>>) {
        let mut documents = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match document {
            Some(document) => documents.insert(window.to_owned(), document),
            None => documents.remove(window),
        };
    }
}

/// What the window needs to show a file it opened.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Opened {
    name: String,
    /// Data rows, without the header; more arrive while `indexing`.
    rows: u64,
    columns: u64,
    /// Column titles from the header row; empty when the file has none.
    header: Vec<String>,
    encoding: String,
    delimiter: String,
    indexing: bool,
    /// Time to open the file and index its first part, in the core.
    open_seconds: f64,
}

/// How far indexing of a large file has come, sent after each step.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Indexing {
    fraction: f64,
    done: bool,
    rows: u64,
    columns: u64,
}

#[tauri::command]
async fn open_file(
    window: WebviewWindow,
    documents: State<'_, Documents>,
    path: String,
    progress: Channel<Indexing>,
) -> Result<Opened, String> {
    let started = Instant::now();
    let document = tauri::async_runtime::spawn_blocking({
        let path = path.clone();
        move || CoreDocument::open_with(path, None, None)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    let open_seconds = started.elapsed().as_secs_f64();

    let info = document.info();
    let header = if info.has_header { document.row(0).unwrap_or_default() } else { Vec::new() };
    let name = Path::new(&path).file_name().map_or(path.clone(), |name| name.to_string_lossy().into_owned());
    let _ = window.set_title(&name);
    documents.set(window.label(), Some(document.clone()));

    let indexing = document.is_indexing();
    if indexing {
        // A window that opens another file meanwhile drops this channel's messages; the old
        // document finishes indexing on its own and is freed.
        std::thread::spawn(move || {
            loop {
                let step = document.continue_indexing(INDEXING_STEP);
                let info = document.info();
                let fraction = step.indexed_bytes as f64 / step.total_bytes.max(1) as f64;
                let message = Indexing { fraction, done: step.done, rows: info.visible_row_count, columns: info.column_count };
                if progress.send(message).is_err() || step.done {
                    break;
                }
            }
        });
    }

    Ok(Opened {
        name,
        rows: info.visible_row_count,
        columns: info.column_count,
        header,
        encoding: info.encoding,
        delimiter: info.delimiter,
        indexing,
        open_seconds,
    })
}

/// Up to `count` data rows from `start`, as the current sort and filters show them.
#[tauri::command]
async fn visible_rows(
    window: WebviewWindow,
    documents: State<'_, Documents>,
    start: u64,
    count: u64,
) -> Result<Vec<Vec<String>>, String> {
    Ok(documents.get(&window)?.visible_rows(start, count))
}

/// What the command line asks for: the file to open, as Explorer and file managers pass it, and
/// with `--bench`, to time scrolling in it, print the times and quit.
#[derive(Serialize)]
struct Launch {
    path: Option<String>,
    bench: bool,
}

#[tauri::command]
fn launch() -> Launch {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    Launch {
        // A missing file is opened too, to say that it is missing.
        path: args.iter().find(|arg| !arg.starts_with('-')).cloned(),
        bench: args.iter().any(|arg| arg == "--bench"),
    }
}

/// Prints a step of a `--bench` run, to see where it stands.
#[tauri::command]
fn bench_step(step: String) {
    eprintln!("bench: {step}");
}

/// Prints the times of a `--bench` run and quits, with a failure status if the run failed.
/// The report also goes to the file `COMMA_BENCH_OUT` names: a Windows release build has no
/// console to print to.
#[tauri::command]
fn bench_done(app: AppHandle, ok: bool, report: String) {
    println!("{report}");
    if let Some(path) = std::env::var_os("COMMA_BENCH_OUT")
        && let Err(error) = std::fs::write(&path, format!("{report}\n"))
    {
        eprintln!("bench: cannot write {}: {error}", Path::new(&path).display());
    }
    app.exit(if ok { 0 } else { 1 });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Documents::default())
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Documents>().set(window.label(), None);
            }
        })
        .invoke_handler(tauri::generate_handler![open_file, visible_rows, launch, bench_step, bench_done])
        .run(tauri::generate_context!())
        .expect("the app could not start");
}

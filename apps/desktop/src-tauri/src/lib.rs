//! The Tauri shell of CommaEditor: the interface sends every command about a document through
//! `core`, which runs it off the main thread with `session::Sessions::dispatch`.
//!
//! Each document has a window. Files to open arrive on the command line, from a second launch
//! (Explorer and file managers start the app again; it hands the files to the running one) and,
//! on macOS, from Finder as `Opened` events; the interface opens them, focusing a window that
//! already shows a file.

pub mod session;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use comma_ffi::SaveFormat;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session::{Failure, Sessions};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

#[tauri::command]
async fn core(
    window: WebviewWindow,
    sessions: State<'_, Arc<Sessions>>,
    command: String,
    args: Value,
) -> Result<Value, Failure> {
    let sessions = sessions.inner().clone();
    let label = window.label().to_owned();
    // COMMA_LOG prints the commands, to follow the interface from a terminal.
    if std::env::var_os("COMMA_LOG").is_some() && command != "rows" && command != "status" {
        eprintln!("{label}: {command} {args}");
    }
    tauri::async_runtime::spawn_blocking(move || sessions.dispatch(&label, &command, args))
        .await
        .map_err(Failure::message)?
}

/// Files to open that came before the interface could take them.
#[derive(Default)]
struct Pending {
    paths: Mutex<Vec<String>>,
    /// The interface asked for them once, and takes later files as events.
    ready: AtomicBool,
    /// Quit was chosen: closing the last window ends the app, also on macOS.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    quitting: AtomicBool,
}

/// The files named on a command line, without its flags, as absolute paths: a second launch
/// names them from its own folder.
fn paths(args: impl IntoIterator<Item = String>, directory: &Path) -> Vec<String> {
    args.into_iter()
        .filter(|arg| !arg.starts_with('-'))
        .map(|arg| directory.join(arg).to_string_lossy().into_owned())
        .collect()
}

/// Hands files to the interface: to the focused window, which opens them; to a new window when
/// there is none, as on macOS after the last one closed.
fn open_files(app: &AppHandle, files: Vec<String>) {
    if files.is_empty() {
        return;
    }
    let pending = app.state::<Pending>();
    if !pending.ready.load(Ordering::Acquire) {
        pending.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).extend(files);
        return;
    }
    let windows = app.webview_windows();
    let target = windows.values().find(|window| window.is_focused().unwrap_or(false)).or_else(|| windows.values().next());
    match target {
        Some(window) => {
            let _ = window.set_focus();
            let _ = window.emit_to(window.label(), "open-files", files);
        }
        None => {
            for file in files {
                new_window(app, Some(&file));
            }
        }
    }
}

/// A window for a document: the file at `path`, or none yet.
fn new_window(app: &AppHandle, path: Option<&str>) {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let label = format!("document-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let url = match path {
        Some(path) => format!("index.html?open={}", urlencoding::encode(path)),
        None => "index.html".to_owned(),
    };
    let _ = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title("CommaEditor")
        .inner_size(1100.0, 720.0)
        .min_inner_size(480.0, 320.0)
        .build();
}

// Drafts: a window's unsaved changes, written a few seconds after each change, so that a crash
// loses little. A clean close removes them; those left at launch are from a session that ended
// otherwise, and the app offers to restore them. Only one app runs, so none is in use then.

/// Files up to this size have drafts; larger ones would take too long to copy every few seconds.
const DRAFT_LARGEST: u64 = 64 << 20;

/// Where the windows of this session keep their drafts, and where those of the last one wait.
struct Drafts {
    current: PathBuf,
    left: PathBuf,
}

/// What a draft belongs to: its file, if it has one, and the format it saves in.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftInfo {
    path: Option<String>,
    format: SaveFormat,
}

/// A draft of the last session, offered at launch.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LeftDraft {
    id: String,
    path: Option<String>,
}

impl Drafts {
    fn new(app: &AppHandle) -> Self {
        let root = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("commaeditor"));
        let drafts = Self { current: root.join("drafts"), left: root.join("drafts-left") };
        // What the last session left behind waits to be restored or discarded.
        if let Ok(entries) = std::fs::read_dir(&drafts.current) {
            let _ = std::fs::create_dir_all(&drafts.left);
            for entry in entries.flatten() {
                let _ = std::fs::rename(entry.path(), drafts.left.join(entry.file_name()));
            }
        }
        let _ = std::fs::create_dir_all(&drafts.current);
        drafts
    }

    fn files(folder: &Path, id: &str) -> (PathBuf, PathBuf) {
        // Window labels are letters, digits and dashes.
        let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        (folder.join(format!("{id}.draft")), folder.join(format!("{id}.json")))
    }

    fn remove(folder: &Path, id: &str) {
        let (data, info) = Self::files(folder, id);
        let _ = std::fs::remove_file(data);
        let _ = std::fs::remove_file(info);
    }

    fn left(&self) -> Vec<LeftDraft> {
        let Ok(entries) = std::fs::read_dir(&self.left) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let id = name.strip_suffix(".json")?.to_owned();
                let info: DraftInfo = serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()?;
                Some(LeftDraft { id, path: info.path })
            })
            .collect()
    }
}

/// Writes the window's draft when it has changes; removes it when it has none.
#[tauri::command]
async fn draft_save(window: WebviewWindow, sessions: State<'_, Arc<Sessions>>, drafts: State<'_, Drafts>) -> Result<(), Failure> {
    let sessions = sessions.inner().clone();
    let label = window.label().to_owned();
    let (data, info) = Drafts::files(&drafts.current, &label);
    tauri::async_runtime::spawn_blocking(move || {
        match sessions.write_draft(&label, &data, DRAFT_LARGEST)? {
            Some((path, format)) => {
                let json = serde_json::to_vec(&DraftInfo { path, format }).map_err(Failure::message)?;
                std::fs::write(&info, json).map_err(Failure::message)
            }
            None => {
                let _ = std::fs::remove_file(&data);
                let _ = std::fs::remove_file(&info);
                Ok(())
            }
        }
    })
    .await
    .map_err(Failure::message)?
}

/// Opens a draft of the last session in this window, and lets it go.
#[tauri::command]
async fn draft_restore(
    window: WebviewWindow,
    sessions: State<'_, Arc<Sessions>>,
    drafts: State<'_, Drafts>,
    id: String,
) -> Result<Value, Failure> {
    let (data, info) = Drafts::files(&drafts.left, &id);
    let folder = drafts.left.clone();
    let sessions = sessions.inner().clone();
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let info: DraftInfo = serde_json::from_slice(&std::fs::read(&info).map_err(Failure::message)?).map_err(Failure::message)?;
        let status = sessions.restore(&label, &data, info.path, info.format)?;
        Drafts::remove(&folder, &id);
        Ok(status)
    })
    .await
    .map_err(Failure::message)?
}

/// Throws a draft of the last session away.
#[tauri::command]
fn draft_discard(drafts: State<'_, Drafts>, id: String) {
    Drafts::remove(&drafts.left, &id);
}

/// What the command line asks for: the files to open, and with `--bench`, to time scrolling in
/// the first, print the times and quit. Also the drafts the last session left.
#[derive(Serialize)]
struct Launch {
    paths: Vec<String>,
    bench: bool,
    drafts: Vec<LeftDraft>,
}

/// Asked once by the first window: the files of the command line and those that came since.
#[tauri::command]
fn launch(pending: State<'_, Pending>, drafts: State<'_, Drafts>) -> Launch {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let bench = args.iter().any(|arg| arg == "--bench");
    let directory = std::env::current_dir().unwrap_or_default();
    let mut files = paths(args, &directory);
    if !pending.ready.swap(true, Ordering::AcqRel) {
        files.extend(pending.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..));
    } else {
        files.clear();
    }
    let left = if bench { Vec::new() } else { drafts.left() };
    Launch { paths: files, bench, drafts: left }
}

/// Closes every window, each asking about its changes, and ends the app with the last.
#[tauri::command]
fn quit(app: AppHandle, pending: State<'_, Pending>) {
    pending.quitting.store(true, Ordering::Release);
    for window in app.webview_windows().values() {
        let _ = window.close();
    }
}

/// Prints an error of the interface to stderr, where it shows when the app runs from a terminal.
#[tauri::command]
fn log(message: String) {
    eprintln!("interface: {message}");
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
    let app = tauri::Builder::default()
        // First: a second launch hands its files to this one and ends.
        .plugin(tauri_plugin_single_instance::init(|app, args, directory| {
            open_files(app, paths(args.into_iter().skip(1), Path::new(&directory)));
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(Sessions::default()))
        .manage(Pending::default())
        .setup(|app| {
            let drafts = Drafts::new(app.handle());
            app.manage(drafts);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Arc<Sessions>>().close(window.label());
                // A window that closed was saved or its changes were dropped.
                Drafts::remove(&window.state::<Drafts>().current, window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            core,
            launch,
            quit,
            log,
            draft_save,
            draft_restore,
            draft_discard,
            bench_step,
            bench_done
        ])
        .build(tauri::generate_context!())
        .expect("the app could not start");
    app.run(on_event);
}

/// macOS sends files to open as events, and keeps apps open without windows.
#[cfg(target_os = "macos")]
fn on_event(app: &AppHandle, event: tauri::RunEvent) {
    use tauri::RunEvent;
    match event {
        // Finder opens files with events rather than arguments.
        RunEvent::Opened { urls } => {
            let files = urls.iter().filter_map(|url| url.to_file_path().ok()).map(|path| path.to_string_lossy().into_owned()).collect();
            open_files(app, files);
        }
        // A Mac app stays open without windows, unless asked to quit.
        RunEvent::ExitRequested { code: None, api, .. } if !app.state::<Pending>().quitting.load(Ordering::Acquire) => {
            api.prevent_exit();
        }
        // Clicking the app in the Dock with no windows opens one.
        RunEvent::Reopen { has_visible_windows: false, .. } => new_window(app, None),
        _ => {}
    }
}

/// Windows and Linux pass files on the command line and end the app with its last window.
#[cfg(not(target_os = "macos"))]
fn on_event(_app: &AppHandle, _event: tauri::RunEvent) {}

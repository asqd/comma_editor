//! The Tauri shell of CommaEditor: the interface sends every command about a document through
//! `core`, which runs it off the main thread with `session::Sessions::dispatch`.
//!
//! Each document has a window. Files to open arrive on the command line, from a second launch
//! (Explorer and file managers start the app again; it hands the files to the running one) and,
//! on macOS, from Finder as `Opened` events; the interface opens them, focusing a window that
//! already shows a file.

pub mod session;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use session::{Failure, Sessions};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

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

/// What the command line asks for: the files to open, and with `--bench`, to time scrolling in
/// the first, print the times and quit.
#[derive(Serialize)]
struct Launch {
    paths: Vec<String>,
    bench: bool,
}

/// Asked once by the first window: the files of the command line and those that came since.
#[tauri::command]
fn launch(pending: State<'_, Pending>) -> Launch {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let bench = args.iter().any(|arg| arg == "--bench");
    let directory = std::env::current_dir().unwrap_or_default();
    let mut files = paths(args, &directory);
    if !pending.ready.swap(true, Ordering::AcqRel) {
        files.extend(pending.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..));
    } else {
        files.clear();
    }
    Launch { paths: files, bench }
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
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Arc<Sessions>>().close(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![core, launch, quit, log, bench_step, bench_done])
        .build(tauri::generate_context!())
        .expect("the app could not start");
    app.run(|app, event| match event {
        // Finder opens files with events rather than arguments.
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let files = urls.iter().filter_map(|url| url.to_file_path().ok()).map(|path| path.to_string_lossy().into_owned()).collect();
            open_files(app, files);
        }
        // A Mac app stays open without windows, unless asked to quit.
        #[cfg(target_os = "macos")]
        RunEvent::ExitRequested { code: None, api, .. } if !app.state::<Pending>().quitting.load(Ordering::Acquire) => {
            api.prevent_exit();
        }
        // Clicking the app in the Dock with no windows opens one.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { has_visible_windows: false, .. } => new_window(app, None),
        _ => {}
    });
}

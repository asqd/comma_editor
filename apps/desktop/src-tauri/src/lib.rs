//! The Tauri shell of CommaEditor: the interface sends every command about a document through
//! `core`, which runs it off the main thread with `session::Sessions::dispatch`.

pub mod session;

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use session::{Failure, Sessions};
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};

#[tauri::command]
async fn core(
    window: WebviewWindow,
    sessions: State<'_, Arc<Sessions>>,
    command: String,
    args: Value,
) -> Result<Value, Failure> {
    let sessions = sessions.inner().clone();
    let label = window.label().to_owned();
    tauri::async_runtime::spawn_blocking(move || sessions.dispatch(&label, &command, args))
        .await
        .map_err(Failure::message)?
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
        .manage(Arc::new(Sessions::default()))
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Arc<Sessions>>().close(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![core, launch, bench_step, bench_done])
        .run(tauri::generate_context!())
        .expect("the app could not start");
}

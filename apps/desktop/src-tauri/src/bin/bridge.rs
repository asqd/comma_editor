//! The development bridge: the commands of the desktop app over HTTP, so that its interface runs
//! in a browser with the real core (`npm run dev`, then http://localhost:1430). Not part of the
//! app; build it with `cargo run --features bridge --bin comma-bridge`.
//!
//! It answers only `POST /core` with a JSON body `{window, command, args}`, from this machine,
//! through the Vite dev server's proxy: requests from other sites in the browser are refused,
//! as they could otherwise read and write files.

use comma_desktop::session::{Failure, Sessions};
use serde::Deserialize;
use serde_json::Value;
use tiny_http::{Header, Method, Response, Server};

const ADDRESS: &str = "127.0.0.1:1431";
/// The Vite dev server, whose proxy is the only way in.
const ORIGIN: &str = "http://localhost:1430";

#[derive(Deserialize)]
struct Call {
    window: String,
    command: String,
    #[serde(default)]
    args: Value,
}

fn header<'a>(request: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    request.headers().iter().find(|header| header.field.equiv(name)).map(|header| header.value.as_str())
}

fn main() {
    let server = Server::http(ADDRESS).expect("the bridge could not listen");
    eprintln!("comma-bridge on http://{ADDRESS}, for {ORIGIN}");
    let sessions = std::sync::Arc::new(Sessions::default());
    // Recipes of the bridge stay apart from the app's.
    sessions.set_recipe_folder(std::env::temp_dir().join("comma-bridge-recipes"));
    for mut request in server.incoming_requests() {
        let host_ok = header(&request, "Host").is_some_and(|host| host == "localhost:1430" || host == ADDRESS);
        let origin_ok = header(&request, "Origin").is_none_or(|origin| origin == ORIGIN);
        let json = header(&request, "Content-Type").is_some_and(|kind| kind.starts_with("application/json"));
        if *request.method() != Method::Post || request.url() != "/core" || !host_ok || !origin_ok || !json {
            let _ = request.respond(Response::from_string("not here").with_status_code(403));
            continue;
        }
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            let _ = request.respond(Response::from_string("unreadable").with_status_code(400));
            continue;
        }
        let sessions = sessions.clone();
        // Each call in a thread of its own, as the app runs them off the main thread.
        std::thread::spawn(move || {
            let (code, reply) = match serde_json::from_str::<Call>(&body) {
                Ok(call) => match sessions.dispatch(&call.window, &call.command, call.args) {
                    Ok(value) => (200, value),
                    Err(failure) => (422, serde_json::to_value(failure).unwrap_or_default()),
                },
                Err(error) => (400, serde_json::to_value(Failure::message(error)).unwrap_or_default()),
            };
            let kind = Header::from_bytes("Content-Type", "application/json").expect("a valid header");
            let _ = request.respond(Response::from_string(reply.to_string()).with_status_code(code).with_header(kind));
        });
    }
}

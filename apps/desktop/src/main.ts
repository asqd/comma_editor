import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./lib/api";

// Errors of the interface go to the app's stderr too: a WebView's console is out of sight.
if (inTauri) {
  const log = (message: string) => void invoke("log", { message }).catch(() => {});
  addEventListener("error", (event) => log(`${event.message} at ${event.filename}:${event.lineno}`));
  addEventListener("unhandledrejection", (event) => log(String(event.reason?.stack ?? event.reason)));
}

mount(App, { target: document.getElementById("app")! });

import { mount } from "svelte";
import App from "./App.svelte";
import DiffWindow from "./DiffWindow.svelte";
import "./app.css";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./lib/api";

// Errors of the interface go to the app's stderr too: a WebView's console is out of sight.
if (inTauri) {
  const log = (message: string) => void invoke("log", { message }).catch(() => {});
  addEventListener("error", (event) => log(`${event.message} at ${event.filename}:${event.lineno}`));
  addEventListener("unhandledrejection", (event) => log(String(event.reason?.stack ?? event.reason)));
}

// A comparison has a window of its own; every other window shows a document.
const parameters = new URLSearchParams(location.search);
const target = document.getElementById("app")!;
if (parameters.has("diff")) {
  mount(DiffWindow, {
    target,
    props: {
      id: Number(parameters.get("diff")),
      source: parameters.get("source") ?? "",
      leftName: parameters.get("left") ?? "",
      otherName: parameters.get("other") ?? "",
    },
  });
} else {
  mount(App, { target });
}

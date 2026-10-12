// Windows of documents: one per document, as in the macOS app. In a browser, tabs stand in for
// them.

import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { call, inTauri } from "./api";

/** Opens a window for the file at `path`, or for a new document. */
export function newWindow(path: string | null): void {
  const query = path ? `?open=${encodeURIComponent(path)}` : "?new=1";
  if (!inTauri) {
    window.open(`${location.pathname}${query}${location.search.includes("lang=") ? `&${location.search.slice(1)}` : ""}`, "_blank");
    return;
  }
  new WebviewWindow(`document-${crypto.randomUUID()}`, {
    url: `index.html${query}`,
    title: "CommaEditor",
    width: 1100,
    height: 720,
    minWidth: 480,
    minHeight: 320,
  });
}

/** Brings forward the window that shows `path`; `false` when none does. */
export async function focusWindowFor(path: string): Promise<boolean> {
  const label = await call<string | null>("window_for_path", { path });
  if (!label) return false;
  if (inTauri) await (await WebviewWindow.getByLabel(label))?.setFocus();
  return true;
}

/** Closes every window, each asking about its changes; the app ends with the last. */
export function quit(): Promise<void> {
  return invoke("quit");
}

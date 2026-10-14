// Writing to the clipboard after the core has made the text: natively in the app, where no
// gesture or permission is needed, and through the browser elsewhere.

import { readText, writeHtml, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { inTauri } from "./api";

export function writeClipboard(text: string): Promise<void> {
  return inTauri ? writeText(text) : navigator.clipboard.writeText(text);
}

/** For Paste in a menu, where no paste event brings the text. */
export function readClipboard(): Promise<string> {
  return inTauri ? readText() : navigator.clipboard.readText();
}

/** HTML with its text beside it, as Copy As ▸ HTML writes in the macOS app. */
export function writeClipboardHtml(html: string, text: string): Promise<void> {
  const page = `<meta charset="utf-8">${html}`;
  if (inTauri) return writeHtml(page, text);
  return navigator.clipboard.write([
    new ClipboardItem({ "text/html": new Blob([page], { type: "text/html" }), "text/plain": new Blob([text], { type: "text/plain" }) }),
  ]);
}

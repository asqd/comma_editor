// Writing to the clipboard after the core has made the text: natively in the app, where no
// gesture or permission is needed, and through the browser elsewhere.

import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { inTauri } from "./api";

export function writeClipboard(text: string): Promise<void> {
  return inTauri ? writeText(text) : navigator.clipboard.writeText(text);
}

/** For Paste in a menu, where no paste event brings the text. */
export function readClipboard(): Promise<string> {
  return inTauri ? readText() : navigator.clipboard.readText();
}

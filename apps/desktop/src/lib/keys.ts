// How a shortcut is written: ⌥⌘↑ on a Mac, Ctrl+Alt+Up elsewhere.

export const mac = navigator.userAgent.includes("Mac");

export interface Shortcut {
  /** `KeyboardEvent.key` for named keys (ArrowUp, Backspace), the letter for letters. */
  key: string;
  /** Cmd on a Mac, Ctrl elsewhere. */
  command?: boolean;
  shift?: boolean;
  alt?: boolean;
}

const macKeys: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Backspace: "⌫",
  Delete: "⌦",
  Enter: "↩",
  Escape: "⎋",
};
const otherKeys: Record<string, string> = {
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
};

export function shortcutLabel(shortcut: Shortcut): string {
  const key = shortcut.key.length === 1 ? shortcut.key.toUpperCase() : shortcut.key;
  if (mac) {
    return `${shortcut.alt ? "⌥" : ""}${shortcut.shift ? "⇧" : ""}${shortcut.command ? "⌘" : ""}${macKeys[key] ?? key}`;
  }
  const parts = [shortcut.command ? "Ctrl" : "", shortcut.alt ? "Alt" : "", shortcut.shift ? "Shift" : "", otherKeys[key] ?? key];
  return parts.filter(Boolean).join("+");
}

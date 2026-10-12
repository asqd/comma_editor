// How a shortcut is written: ⌥⌘↑ on a Mac, Ctrl+Alt+Up elsewhere.

export const mac = navigator.userAgent.includes("Mac");

export interface Shortcut {
  /** `KeyboardEvent.key` for named keys (ArrowUp, Backspace), the letter for letters. */
  key: string;
  /** Cmd on a Mac, Ctrl elsewhere. */
  command?: boolean;
  shift?: boolean;
  alt?: boolean;
  /**
   * ⌃ with ⌘ on a Mac; Ctrl+Shift with arrows already extends the selection elsewhere, so there
   * it is Alt and Shift with Ctrl.
   */
  control?: boolean;
}

/** The modifiers a shortcut takes on this system. */
export function modifiers(shortcut: Shortcut): { command: boolean; shift: boolean; alt: boolean; control: boolean } {
  if (!shortcut.control) return { command: !!shortcut.command, shift: !!shortcut.shift, alt: !!shortcut.alt, control: false };
  return mac
    ? { command: !!shortcut.command, shift: !!shortcut.shift, alt: !!shortcut.alt, control: true }
    : { command: !!shortcut.command, shift: true, alt: true, control: false };
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
  const held = modifiers(shortcut);
  if (mac) {
    return `${held.control ? "⌃" : ""}${held.alt ? "⌥" : ""}${held.shift ? "⇧" : ""}${held.command ? "⌘" : ""}${macKeys[key] ?? key}`;
  }
  const parts = [held.command ? "Ctrl" : "", held.alt ? "Alt" : "", held.shift ? "Shift" : "", otherKeys[key] ?? key];
  return parts.filter(Boolean).join("+");
}

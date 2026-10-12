// The app's commands: what each does, when it can, and its shortcut. The menus show them and
// the keyboard runs them, through `Commands`, so that a key runs a command once whether the
// page or the menu sees it first.

import { mac, type Shortcut } from "./keys";

export interface Command {
  id: string;
  label: string | (() => string);
  shortcut?: Shortcut;
  /**
   * Its shortcut works while a text field has the focus too; otherwise the field keeps the key
   * (Cmd-Backspace deletes text there, not rows).
   */
  inFields?: boolean;
  enabled?: () => boolean;
  checked?: () => boolean;
  run: () => void;
}

/** How long a command run by its key ignores the same command from the menu, which may follow. */
const ECHO_MS = 400;

export const labelOf = (command: Command) => (typeof command.label === "function" ? command.label() : command.label);

/** Whether a text field other than the grid's own has the focus. */
export function fieldFocused(): boolean {
  const element = document.activeElement;
  if (!element || element.classList.contains("grid-keys")) return false;
  return element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement;
}

/** Whether `event` is `shortcut`; letters by the key's place, so that any layout works. */
export function matches(event: KeyboardEvent, shortcut: Shortcut): boolean {
  const command = mac ? event.metaKey : event.ctrlKey;
  const other = mac ? event.ctrlKey : event.metaKey;
  if (command !== !!shortcut.command || event.shiftKey !== !!shortcut.shift || event.altKey !== !!shortcut.alt || other) return false;
  // Punctuation by the key's place too: the keys of [ and ] type х and ъ on a Russian layout.
  const places: Record<string, string> = { "[": "BracketLeft", "]": "BracketRight", "=": "Equal", "-": "Minus", ",": "Comma", ".": "Period" };
  // Some keyboards, on screen or remote, give no place: then the character decides.
  if (!event.code) return event.key.toLowerCase() === shortcut.key.toLowerCase();
  if (places[shortcut.key]) return event.code === places[shortcut.key];
  if (shortcut.key.length === 1) return event.code === `Key${shortcut.key.toUpperCase()}` || event.code === `Digit${shortcut.key}`;
  return event.key === shortcut.key;
}

export class Commands {
  private readonly byId = new Map<string, Command>();
  private readonly keyed = new Map<string, number>();

  constructor(readonly list: Command[]) {
    for (const command of list) this.byId.set(command.id, command);
  }

  get(id: string): Command | undefined {
    return this.byId.get(id);
  }

  /** Runs the command a key stands for; `false` when the key is not a command's. */
  key(event: KeyboardEvent): boolean {
    const command = this.list.find((command) => command.shortcut && matches(event, command.shortcut));
    if (!command) return false;
    this.keyed.set(command.id, performance.now());
    // The grid took the key itself, as it does Cmd-A.
    if (event.defaultPrevented) return true;
    if (fieldFocused() && !command.inFields) return false;
    event.preventDefault();
    if (command.enabled?.() ?? true) command.run();
    return true;
  }

  /** Runs a command chosen in a menu, unless its key just ran it. */
  menu(id: string): void {
    const keyed = this.keyed.get(id);
    if (keyed !== undefined && performance.now() - keyed < ECHO_MS) return;
    const command = this.byId.get(id);
    if (command && (command.enabled?.() ?? true)) command.run();
  }
}

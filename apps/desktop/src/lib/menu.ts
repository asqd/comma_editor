// The native menu bar, built from the commands: the app's menu on macOS, kept for the window in
// front, and each window's own menu bar on Windows and Linux.

import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { labelOf, type Commands } from "./commands";
import { mac, type Shortcut } from "./keys";

/** An entry of a menu: a command, a separator, a submenu, or an item the system provides. */
export type Entry =
  | string
  | "-"
  | { title: string; entries: Entry[] }
  | { native: "Cut" | "Copy" | "Paste" | "Minimize" | "Maximize" | "Fullscreen" | "Hide" | "HideOthers" | "ShowAll" | "Services"; text?: string }
  | { about: string; version: string; text: string };

function accelerator(shortcut: Shortcut): string {
  const keys: Record<string, string> = { ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right" };
  const key = shortcut.key.length === 1 ? shortcut.key.toUpperCase() : (keys[shortcut.key] ?? shortcut.key);
  return [shortcut.command ? "CmdOrCtrl" : "", shortcut.alt ? "Alt" : "", shortcut.shift ? "Shift" : "", key].filter(Boolean).join("+");
}

export class NativeMenu {
  private menu: Menu | null = null;
  private readonly items = new Map<string, MenuItem | CheckMenuItem>();
  private state = new Map<string, string>();

  constructor(private readonly commands: Commands) {}

  /** Builds the menu bar anew, as when the recent files change. */
  async install(entries: { title: string; entries: Entry[] }[]): Promise<void> {
    this.items.clear();
    this.state.clear();
    const submenus = await Promise.all(entries.map((entry) => this.submenu(entry)));
    const menu = await Menu.new({ items: submenus });
    this.menu = menu;
    if (mac) await menu.setAsAppMenu();
    else await menu.setAsWindowMenu(getCurrentWindow());
    await this.refresh();
  }

  /** Brings the app's menu to this window: macOS has one menu bar for every window. */
  async focus(): Promise<void> {
    if (mac && this.menu) await this.menu.setAsAppMenu();
  }

  /** Enables, checks and names the items as the commands now say. */
  async refresh(): Promise<void> {
    const changes: Promise<unknown>[] = [];
    for (const [id, item] of this.items) {
      const command = this.commands.get(id);
      if (!command) continue;
      const enabled = command.enabled?.() ?? true;
      const checked = command.checked?.() ?? false;
      const text = labelOf(command);
      const state = `${enabled}|${checked}|${text}`;
      if (this.state.get(id) === state) continue;
      this.state.set(id, state);
      changes.push(item.setEnabled(enabled), item.setText(text));
      if (item instanceof CheckMenuItem) changes.push(item.setChecked(checked));
    }
    await Promise.all(changes);
  }

  private async submenu(entry: { title: string; entries: Entry[] }): Promise<Submenu> {
    const items = await Promise.all(entry.entries.map((child) => this.entry(child)));
    return Submenu.new({ text: entry.title, items });
  }

  private async entry(entry: Entry): Promise<MenuItem | CheckMenuItem | PredefinedMenuItem | Submenu> {
    if (entry === "-") return PredefinedMenuItem.new({ item: "Separator" });
    if (typeof entry === "object" && "native" in entry) return PredefinedMenuItem.new({ item: entry.native, text: entry.text });
    if (typeof entry === "object" && "about" in entry) {
      return PredefinedMenuItem.new({ item: { About: { name: entry.about, version: entry.version } }, text: entry.text });
    }
    if (typeof entry === "object") return this.submenu(entry);
    const command = this.commands.get(entry);
    if (!command) throw new Error(`no command ${entry}`);
    const options = {
      id: command.id,
      text: labelOf(command),
      accelerator: command.shortcut ? accelerator(command.shortcut) : undefined,
      action: () => this.commands.menu(command.id),
    };
    const item = command.checked ? await CheckMenuItem.new({ ...options, checked: command.checked() }) : await MenuItem.new(options);
    this.items.set(command.id, item);
    return item;
  }
}

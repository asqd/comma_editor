<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open, save as chooseSavePath } from "@tauri-apps/plugin-dialog";
  import { Grid, type ColumnLayout, type TableSource } from "./grid/Grid";
  import { formatBench, runBench } from "./grid/bench";
  import {
    call,
    CoreFailure,
    inTauri,
    type ColumnProfile,
    type EditResult,
    type Problem,
    type ProblemKind,
    type RepairFix,
    type RepairOption,
    type RepairReport, type TextEncoding, type FilterRule, type RangeStats, type SaveFormat, type SheetLayout, type SortKey, type Status, type ViewSpec } from "./lib/api";
  import { Selection, type Cell } from "./grid/selection";
  import { CellEditor, type Move } from "./grid/editor";
  import CellSheet from "./CellSheet.svelte";
  import Alert, { type Button } from "./Alert.svelte";
  import SaveSheet from "./SaveSheet.svelte";
  import FindBar from "./FindBar.svelte";
  import FilterBar from "./FilterBar.svelte";
  import OpenOptions from "./OpenOptions.svelte";
  import ProfilePanel from "./ProfilePanel.svelte";
  import RepairPanel from "./RepairPanel.svelte";
  import RecordCard, { type Field } from "./RecordCard.svelte";
  import FormulaSheet from "./FormulaSheet.svelte";
  import RecipesSheet from "./RecipesSheet.svelte";
  import SheetTabs from "./SheetTabs.svelte";
  import { sheetColumns, workbookLocale } from "./lib/workbook";
  import type { Recipe } from "./lib/recipes";
  import { describeFormula, type FormulaReport } from "./lib/formula";
  import { delimiterName, delimiters, encodingName, encodings, kindName } from "./lib/formats";
  import { Document } from "./lib/document.svelte";
  import { describe } from "./lib/errors";
  import { L, count, locale } from "./lib/i18n";
  import { readClipboard, writeClipboard, writeClipboardHtml } from "./lib/clipboard";
  import { appName, downloadPage, shortcutLabel } from "./lib/keys";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import Toolbar, { type ToolbarItem } from "./Toolbar.svelte";
  import { Commands, fieldFocused, labelOf, type Command } from "./lib/commands";
  import { NativeMenu, type Entry } from "./lib/menu";
  import { addRecent, clearRecent, recentFiles } from "./lib/recent";
  import { diffWindow, focusWindowFor, newWindow, quit } from "./lib/windows";
  import { invoke } from "@tauri-apps/api/core";
  import type { CellRange } from "./lib/api";

  const mac = navigator.userAgent.includes("Mac");
  const openShortcut = shortcutLabel({ key: "o", command: true });

  let host: HTMLElement;
  let grid: Grid | null = null;
  let editor: CellEditor | null = null;
  /** A question or a problem shown over the table, and who waits for the answer. */
  let alert = $state<{
    message: string;
    detail?: string;
    buttons: Button[];
    field?: string;
    fieldLabel?: string;
    placeholder?: string;
    checkbox?: string;
    answer: (value: string, text: string, checked: boolean) => void;
  } | null>(null);
  /** The panel at the right of the table, as in the macOS app. */
  let panel = $state<"profile" | "repair" | "card" | null>(null);
  /** The record card: the active row as a form. */
  let card = $state.raw<{ row: number; title: string; position: string; fields: Field[] } | null>(null);
  /** File repair: the last check, the selected problem, its row as the file has it and its fixes. */
  let repair = $state.raw<RepairReport | null>(null);
  let repairLoading = $state(false);
  let repairRow = $state<number | null>(null);
  let repairRaw = $state("");
  let repairOptions = $state.raw<RepairOption[]>([]);
  let garbledExample = $state.raw<[string, string] | null>(null);
  let profile = $state.raw<ColumnProfile | null>(null);
  let profiling = $state(false);
  let profiledColumn = $state<number | null>(null);
  let profiledName = $state("");
  /** The saved recipes, and whether their window is open. */
  let recipes = $state.raw<Recipe[]>([]);
  let managing = $state(false);
  /** The computed column window, for the active column. */
  let computing = $state<{ column: number; name: string; columns: string[] } | null>(null);
  /** What the background change in progress does, for the status bar. */
  let editing = $state("");
  /** The format of a file being saved, asked before the file is chosen. */
  let saving = $state<{ format: SaveFormat; title?: string; excel?: boolean; answer: (format: SaveFormat | null) => void } | null>(null);
  /** Exports writing their files, for the status bar. */
  let exporting = $state(0);
  /** The find bar, with the replace row or not, and the selection it may search in. */
  let finding = $state<{ replacing: boolean; selection: CellRange | null } | null>(null);
  let findBar = $state<ReturnType<typeof FindBar>>();
  /** The filter bar, and why its filters cannot apply. */
  let filtering = $state(false);
  let filterBar = $state<ReturnType<typeof FilterBar>>();
  let filterProblem = $state("");
  /** A menu open at the pointer. */
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  /** A value with line breaks being edited in a window of its own. */
  let sheet = $state<{ cell: Cell; heading: string; value: string } | null>(null);
  /** The status the view shows rows for; the rows reload when it differs in what they show. */
  const doc = new Document((status, previous) => {
    if (!grid) return;
    // A file opened, perhaps again: the table and what was found in it start over.
    const opened = !previous || previous.path !== status.path || fresh;
    // Another sheet of the workbook: the table is built anew, as it was left on that sheet or
    // as the file lays it out.
    const sheetShown = !opened && !!status.book && previous.book?.index !== status.book.index;
    grid.marksMissing = !status.isSheet;
    if (opened) {
      fresh = false;
      sheetStates.clear();
      grid.setSource(table);
      if (status.book) void showSheetState(status, null);
    } else if (sheetShown) {
      sheetStates.set(previous.book!.index, currentSheetState(previous));
      grid.setSource(table);
      void showSheetState(status, sheetStates.get(status.book!.index) ?? null);
    } else if (
      previous.change !== status.change ||
      previous.revision !== status.revision ||
      previous.hasHeader !== status.hasHeader ||
      JSON.stringify(previous.view) !== JSON.stringify(status.view)
    )
      grid.reload();
    else if (previous.rows !== status.rows || previous.columns !== status.columns) grid.update();
    grid.setView(status.view);
    restoreLayout(previous, status);
    void showTitle(status);
    if (previous?.change !== status.change || previous?.dirty !== status.dirty) keepDraft();
    if (previous?.change !== status.change || JSON.stringify(previous?.view) !== JSON.stringify(status.view)) {
      refreshProfile();
      void refreshCard();
    }
    // Broken rows are looked for after every change, and once a large file is read whole.
    if (opened || sheetShown) repair = null;
    if (!previous || sheetShown || previous.change !== status.change || previous.hasHeader !== status.hasHeader || (previous.indexing !== null && status.indexing === null))
      scheduleProblemCheck(previous ? 300 : 0);
    if (opened || sheetShown || previous?.change !== status.change || previous?.revision !== status.revision) void inspectCell();
  });

  // Sheets of a workbook (GridViewController+Sheets.swift): the table's state on each sheet as
  // one moves between them, and the layout Excel gives each sheet at first.

  /** How the table looked on a sheet, to show it the same way on coming back. */
  interface SheetViewState {
    layout: ColumnLayout;
    frozenColumns: number;
    /** Rows of the sheet frozen at the top, its first row counted, whatever the mode. */
    frozenTop: number;
    selection: Selection | null;
    top: number;
    left: number;
  }
  const sheetStates = new Map<number, SheetViewState>();

  function currentSheetState(status: Status): SheetViewState {
    return {
      layout: grid!.columnLayout,
      frozenColumns: grid!.frozenColumns,
      frozenTop: grid!.frozenRows + (status.hasHeader ? 1 : 0),
      selection: grid!.selection,
      top: grid!.scrollTop,
      left: grid!.scrollLeft,
    };
  }

  /** Lays out the sheet shown: as it was left, or as Excel shows it. */
  async function showSheetState(status: Status, state: SheetViewState | null) {
    if (!grid) return;
    const index = status.book?.index;
    const offset = status.hasHeader ? 1 : 0;
    if (!state) {
      const layout = await call<SheetLayout | null>("sheet_layout").catch(() => null);
      const shown = doc.status;
      if (!grid || !shown || shown.book?.index !== index) return;
      const columns = shown.columns;
      grid.columnLayout = sheetColumns(layout, columns, 140);
      const first = grid.shownColumn(0) ?? 0;
      state = {
        layout: grid.columnLayout,
        frozenColumns: Math.min(layout?.frozenColumns ?? 0, Math.max(0, columns - 1)),
        frozenTop: layout?.frozenRows ?? 0,
        selection: shown.rows > 0 ? Selection.at({ row: 0, column: first }) : null,
        top: 0,
        left: 0,
      };
    }
    grid.columnLayout = state.layout;
    grid.freezeColumns(state.frozenColumns);
    grid.freezeRows(Math.min(Math.max(0, state.frozenTop - offset), Math.max(0, (doc.status?.rows ?? 0) - 1)));
    grid.select(state.selection, false);
    grid.scrollTo(state.top, state.left);
    layoutChanged += 1;
  }

  /** Shows a sheet of the workbook; one not read before is read first, its progress shown. */
  async function chooseSheet(index: number) {
    const book = doc.status?.book;
    if (!book || index === book.index || editor?.editing) return;
    await doc.attempt(() => doc.run("show_sheet", { index }));
  }

  /** The formula of the active cell of a sheet, for the status bar. */
  let activeFormula = $state("");
  let inspected = 0;
  async function inspectCell() {
    const asked = ++inspected;
    const cursor = grid?.selection;
    if (!doc.status?.isSheet || !cursor || !cursor.single) {
      activeFormula = "";
      return;
    }
    const text = await call<{ formula: string | null }>("cell_text", { row: cursor.cursor.row, column: cursor.cursor.column }).catch(() => null);
    if (asked === inspected) activeFormula = text?.formula ?? "";
  }

  /**
   * Column widths and hidden columns before and after a change that inserted or deleted a
   * column, by the change ids around it: undo and redo bring them back, as in the macOS app.
   */
  const layouts: { sheet: number | null; before: number | null; after: number | null; old: ColumnLayout; next: ColumnLayout }[] = [];

  function restoreLayout(previous: Status | null, status: Status) {
    if (!grid || !previous || previous.book?.index !== status.book?.index) return;
    for (const entry of layouts) {
      if (entry.sheet !== (status.book?.index ?? null)) continue;
      if (previous.change === entry.after && status.change === entry.before) grid.columnLayout = entry.old;
      else if (previous.change === entry.before && status.change === entry.after) grid.columnLayout = entry.next;
    }
  }

  /** Runs a change to the columns, shifting the layout with `shift` and keeping it for undo. */
  async function changeColumns(work: () => Promise<unknown>, shift: () => void): Promise<boolean> {
    if (!grid || !doc.status) return false;
    const before = doc.status.change;
    const old = grid.columnLayout;
    if (!(await doc.attempt(work))) return false;
    shift();
    grid.update();
    layouts.push({ sheet: doc.status.book?.index ?? null, before, after: doc.status.change, old, next: grid.columnLayout });
    return true;
  }

  /** Writes the draft a few seconds after the last change, or removes it once saved. */
  let draftTimer = 0;
  function keepDraft() {
    if (!inTauri || benching) return;
    clearTimeout(draftTimer);
    draftTimer = window.setTimeout(() => void invoke("draft_save").catch(() => {}), 3000);
  }

  /** Opens a draft of the last session in this window. */
  async function restoreDraft(id: string) {
    fresh = true;
    try {
      doc.show(await invoke<Status>("draft_restore", { id }));
    } catch (error) {
      failure = describe(error);
    }
  }

  /** Offers each draft the last session left: restored ones open, the first in this window. */
  async function offerDrafts(drafts: { id: string; path: string | null }[]) {
    for (const draft of drafts) {
      const answer = await ask(
        L("Restore the unsaved changes to “%@”?", name(draft.path)),
        L("CommaEditor closed before they were saved. Restored changes open in a window; save them to keep them."),
        [
          { label: L("Discard"), value: "discard", cancel: true },
          { label: L("Restore"), value: "restore", default: true },
        ],
      );
      if (answer === "discard") void invoke("draft_discard", { id: draft.id });
      else if (!doc.open) await restoreDraft(draft.id);
      else newWindow(null, draft.id);
    }
  }
  const table: TableSource = {
    get rows() {
      return doc.status?.rows ?? 0;
    },
    get columns() {
      return doc.status?.columns ?? 0;
    },
    get header() {
      return doc.status?.header ?? [];
    },
    fetch: (start, count) => call<{ rows: string[][]; numbers: number[] }>("rows", { start, count }),
  };

  /** A file was just opened, perhaps again: the grid starts over. */
  let fresh = false;
  /** How the file was last opened: chosen encoding and delimiter, `null` detected. */
  let reading = $state.raw<{ encoding: TextEncoding | null; delimiter: string | null }>({ encoding: null, delimiter: null });
  /** The encoding and delimiter window, and whether it opened by itself. */
  let checking = $state<{ uncertain: boolean } | null>(null);
  let askedAboutDetection = false;
  let failure = $state<{ message: string; detail?: string } | null>(null);
  let openSeconds = 0;
  let firstScreen: number | null = null;
  let benchText = $state("");
  let selection = $state.raw<Selection | null>(null);
  let stats = $state.raw<RangeStats | null>(null);
  let statsFor = 0;

  /** Counts, sums and averages the selected cells, as the macOS app does up to 2 million. */
  async function measure(selected: Selection | null) {
    const asked = ++statsFor;
    stats = null;
    if (!selected || selected.single || selected.rowCount * selected.columnCount > 2_000_000) return;
    try {
      const result = await call<RangeStats>("range_stats", { range: selected.range });
      if (asked === statsFor) stats = result;
    } catch {
      // The status bar simply goes without the sum.
    }
  }

  const name = (path: string | null) => (path ? (path.split(/[\\/]/).pop() ?? path) : L("Untitled"));

  async function showTitle(status: Status) {
    const title = status.dirty ? `${name(status.path)} — ${L("Edited")}` : name(status.path);
    if (inTauri) await getCurrentWindow().setTitle(title);
    else document.title = title;
  }

  const status = $derived.by(() => {
    const status = doc.status;
    if (!status) return "";
    const parts: string[] = [];
    if (status.recording !== null) parts.push(`● ${L("Recording a recipe: %@ steps", count(status.recording))}`);
    if (selection && !selection.single) {
      parts.push(L("Selected: %@ × %@", count(selection.rowCount), count(selection.columnCount)));
      if (stats && stats.numbers > 0) {
        const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 6 });
        parts.push(L("Sum: %@", number.format(stats.sum)));
        parts.push(L("Average: %@", number.format(stats.sum / stats.numbers)));
      }
    }
    else if (activeFormula) parts.push(`ƒ  ${activeFormula.replace(/\r\n|\r|\n/g, " ")}`);
    if (status.book && status.book.circular > 0) parts.unshift(`⚠︎ ${L("Circular references: %@", count(status.book.circular))}`);
    if (status.view.filters.length > 0) parts.push(L("Showing %@ of %@", count(status.rows), count(status.dataRows)));
    else parts.push(L("Rows: %@", count(status.rows)));
    parts.push(L("Columns: %@", count(status.columns)));
    if (status.isSheet) {
      parts.push(status.book?.legacy ? L("Excel 97–2003 workbook, view only") : L("Excel workbook"));
    } else {
      parts.push(status.hasBom ? L("%@ with BOM", status.encoding) : status.encoding);
      const delimiter = status.delimiter === "\t" ? L("Tab") : status.delimiter === " " ? L("Space") : status.delimiter;
      parts.push(L("Delimiter: %@", delimiter));
      parts.push({ lf: "LF", crLf: "CRLF", cr: "CR" }[status.lineEnding]);
    }
    void layoutChanged;
    const hidden = grid?.hiddenCount ?? 0;
    if (hidden > 0) parts.push(L("Hidden columns: %@", count(hidden)));
    if (status.view.sort.length > 0) {
      const keys = status.view.sort.map((key) => `${grid?.title(key.column) ?? key.column} ${key.ascending ? "↑" : "↓"}`);
      parts.push(L("Sorted by %@", keys.join(", ")));
    }
    if (status.book && status.book.notComputed > 0) parts.push(L("Formulas Excel will compute: %@", count(status.book.notComputed)));
    return parts.join("   ·   ");
  });

  const busy = $derived.by(() => {
    const status = doc.status;
    if (!status) return "";
    const loading = status.book?.loading;
    if (loading) return L("Reading “%@”: %@", status.book!.sheets[loading.index]?.name ?? "", `${Math.floor(loading.fraction * 100)} %`);
    if (status.indexing !== null) return L("Reading the file: %@", `${Math.floor(status.indexing * 100)} %`);
    if (exporting > 0) return L("Exporting…");
    if (status.job?.kind === "view") return L("Sorting and filtering…");
    if (status.job?.kind === "edit") return `${editing || L("Replacing…")} ${Math.floor((status.job.fraction ?? 0) * 100)} %`;
    return "";
  });

  /**
   * Opens a file in this window, with an encoding and a delimiter or detecting them; the time
   * to the first screen goes to the bench.
   */
  async function show(path: string, options = { encoding: null as TextEncoding | null, delimiter: null as string | null }): Promise<boolean> {
    failure = null;
    fresh = true;
    firstScreen = null;
    const started = performance.now();
    if (grid) {
      grid.onFrame = (complete) => {
        if (complete && firstScreen === null && doc.open) firstScreen = performance.now() - started;
      };
    }
    try {
      const status = await doc.run("open", { path, ...options, locale: workbookLocale() });
      openSeconds = (performance.now() - started) / 1000;
      reading = options;
      addRecent(path);
      void installMenu();
      // Once per window, as in the macOS app: a guessed encoding or delimiter is shown to check.
      if (!status.isSheet && !status.detectionConfident && !askedAboutDetection && !benching) {
        askedAboutDetection = true;
        checking = { uncertain: true };
      }
      return true;
    } catch (error) {
      failure = describe(error);
      return false;
    }
  }

  async function choose() {
    const path = inTauri
      ? await open({
          multiple: false,
          directory: false,
          filters: [
            { name: L("CSV and Excel"), extensions: ["csv", "tsv", "tab", "txt", "xlsx", "xlsm", "xls"] },
            { name: "CSV", extensions: ["csv", "tsv", "tab", "txt"] },
            { name: L("Excel workbook"), extensions: ["xlsx", "xlsm", "xls"] },
            { name: "*", extensions: ["*"] },
          ],
        })
      : await askText(L("Open…"), doc.status?.path ?? "", L("Open"));
    if (path) await openPaths([path]);
  }

  /**
   * Opens files: one shown in another window brings it forward, the first goes into this window
   * while it has no document, the others into windows of their own.
   */
  async function openPaths(paths: string[]) {
    for (const path of paths) {
      if (await focusWindowFor(path).catch(() => false)) continue;
      if (!doc.open && !opening) {
        opening = true;
        await show(path);
        opening = false;
      } else {
        newWindow(path);
      }
    }
  }
  let opening = false;

  /** Reads the file again with another encoding or delimiter, losing changes once asked. */
  async function reopen(encoding: TextEncoding | null, delimiter: string | null) {
    const status = doc.status;
    if (!status?.path) return;
    if (status.dirty) {
      const answer = await ask(
        L("Reopen the file and lose unsaved changes?"),
        L("The file is read again from disk; changes made since the last save are lost."),
        [
          { label: L("Cancel"), value: "cancel", cancel: true },
          { label: L("Reopen"), value: "reopen", default: true },
        ],
      );
      if (answer !== "reopen") return;
    }
    await show(status.path, { encoding, delimiter });
  }

  /** This window's name, by which a comparison shows rows in it. */
  const windowName = () => (inTauri ? getCurrentWindow().label : (sessionStorage.getItem("comma-window") ?? ""));

  /** File ▸ Compare with File: chooses the other file and opens the comparison's window. */
  async function compareWithFile() {
    const status = doc.status;
    if (!status || status.indexing !== null) return;
    const message = L("Choose a file to compare with “%@”.", name(status.path));
    const path = inTauri
      ? await open({ multiple: false, directory: false, title: message, defaultPath: status.path?.replace(/[^\\/]*$/, "") })
      : await askText(message, status.path?.replace(/[^\\/]*$/, "") ?? "", L("Compare"));
    if (!path) return;
    try {
      const { id } = await call<{ id: number }>("diff_start", { path, locale: workbookLocale().display });
      diffWindow(id, windowName(), name(status.path), name(path), L("%@ and %@", name(status.path), name(path)));
    } catch (error) {
      await report(error);
    }
  }

  /** Selects a row of the file a comparison points at, if it shows. */
  async function revealRow(record: number) {
    const row = await call<number | null>("visible_index", { row: record });
    if (row === null || !grid) return;
    grid.select(Selection.at({ row, column: grid.selection?.cursor.column ?? 0 }));
    grid.focus();
  }

  /** A new document: in this window while it has none, in a new one otherwise. */
  async function newDocument() {
    if (doc.open) newWindow(null);
    else await doc.attempt(() => doc.run("new"));
  }

  async function bench(): Promise<string> {
    if (!grid || !doc.status) return "";
    const result = await runBench(grid);
    const where = [
      navigator.userAgent,
      `${name(doc.status.path)}: ${count(doc.status.rows)}, ${innerWidth}×${innerHeight}, ×${devicePixelRatio}`,
      [
        L("opened in %@ s", openSeconds.toFixed(3)),
        firstScreen === null ? "" : L("first screen in %@ s", (firstScreen / 1000).toFixed(3)),
      ]
        .filter(Boolean)
        .join(", "),
    ].join("\n");
    return formatBench(result, where);
  }

  /** `--bench FILE`: opens the file, waits for it to be read whole, times scrolling and quits. */
  let benching = false;
  async function benchAndQuit(path: string) {
    benching = true;
    const { benchDone, benchStep } = await import("./lib/bench-launch");
    void benchStep(`opening ${path}`);
    if (!(await show(path))) {
      await benchDone(false, `${path}: ${failure?.message ?? ""} ${failure?.detail ?? ""}`);
      return;
    }
    void benchStep(`opened, ${doc.status?.rows} rows so far; reading the rest`);
    while (doc.status?.indexing !== null || firstScreen === null) await new Promise((resolve) => setTimeout(resolve, 50));
    void benchStep(`read ${doc.status?.rows} rows; scrolling`);
    try {
      await benchDone(true, await bench());
    } catch (problem) {
      await benchDone(false, `bench failed: ${problem}`);
    }
  }

  function ask(message: string, detail: string | undefined, buttons: Button[]): Promise<string> {
    return new Promise((answer) => (alert = { message, detail, buttons, answer }));
  }

  /** Asks for a line of text; `null` when cancelled. */
  function askText(message: string, field: string, action: string): Promise<string | null> {
    const buttons = [
      { label: L("Cancel"), value: "cancel", cancel: true },
      { label: action, value: "ok", default: true },
    ];
    return new Promise((answer) => (alert = { message, buttons, field, answer: (value, text) => answer(value === "ok" ? text : null) }));
  }

  /**
   * The macOS app's question with a field and a checkbox (GridViewController+Columns.ask):
   * `null` when cancelled.
   */
  function askForm(
    message: string,
    detail: string,
    options: { field?: string; fieldLabel?: string; placeholder?: string; checkbox?: string },
  ): Promise<{ text: string; checked: boolean } | null> {
    const buttons = [
      { label: L("Cancel"), value: "cancel", cancel: true },
      { label: L("OK"), value: "ok", default: true },
    ];
    return new Promise(
      (answer) =>
        (alert = { message, detail, buttons, ...options, answer: (value, text, checked) => answer(value === "ok" ? { text, checked } : null) }),
    );
  }

  function inform(message: string, detail?: string): Promise<string> {
    return ask(message, detail, [{ label: L("OK"), value: "ok", default: true, cancel: true }]);
  }

  function report(error: unknown): Promise<string> {
    const problem = describe(error);
    return ask(problem.message, problem.detail, [{ label: L("OK"), value: "ok", default: true, cancel: true }]);
  }

  /** Saves to the document's file, or asks where to when it has none. */
  async function save(): Promise<boolean> {
    const status = doc.status;
    if (!status) return false;
    if (!status.path) return saveAs();
    try {
      await doc.run("save");
      return true;
    } catch (error) {
      await report(error);
      return false;
    }
  }

  /** Asks for the format and then the file, and saves in that format from now on. */
  async function saveAs(): Promise<boolean> {
    if (!doc.status) return false;
    if (doc.status.book) return saveBookAs();
    const current = await call<SaveFormat>("save_format");
    const format = await new Promise<SaveFormat | null>((answer) => (saving = { format: current, answer }));
    if (!format) return false;
    const extension = format.delimiter === "\t" ? "tsv" : "csv";
    const base = name(doc.status.path).replace(/\.[^.]*$/, "");
    const path = inTauri
      ? await chooseSavePath({ defaultPath: `${base}.${extension}`, filters: [{ name: extension.toUpperCase(), extensions: [extension] }] })
      : await askText(L("Save As…"), doc.status.path ?? `${base}.${extension}`, L("Save"));
    if (!path) return false;
    try {
      await doc.run("save_as", { path, format });
      addRecent(path);
      void installMenu();
      return true;
    } catch (error) {
      await report(error);
      return false;
    }
  }

  /** A workbook saves as itself, in its own format: no CSV options (WorkbookDocument.saveAs). */
  async function saveBookAs(): Promise<boolean> {
    const status = doc.status;
    if (!status?.book || status.book.legacy) return false;
    const extension = /\.xlsm$/i.test(status.path ?? "") ? "xlsm" : "xlsx";
    const path = inTauri
      ? await chooseSavePath({ defaultPath: name(status.path), filters: [{ name: L("Excel workbook"), extensions: [extension] }] })
      : await askText(L("Save As…"), status.path ?? `${name(status.path)}.${extension}`, L("Save"));
    if (!path) return false;
    try {
      await doc.run("save_book_as", { path });
      addRecent(path);
      void installMenu();
      return true;
    } catch (error) {
      await report(error);
      return false;
    }
  }

  // Export (GridViewController+Export.swift and CSVDocument.saveTo).

  type ExportKind = "json" | "markdown" | "html" | "sql" | "xlsx";
  const extensions: Record<ExportKind, string> = { json: "json", markdown: "md", html: "html", sql: "sql", xlsx: "xlsx" };

  /** The file's name without its extension: the HTML title, SQL table and sheet of exports. */
  const exportName = () => (doc.status?.path ? name(doc.status.path).replace(/\.[^.]*$/, "") : "data");

  /** Asks where a file goes; in a browser, in a field. */
  async function chooseTarget(defaultName: string, extension: string, title?: string): Promise<string | null> {
    if (inTauri) return chooseSavePath({ title, defaultPath: defaultName, filters: [{ name: extension.toUpperCase(), extensions: [extension] }] });
    const folder = doc.status?.path?.replace(/[^\\/]*$/, "") ?? "";
    return askText(title ?? L("Export…"), `${folder}${defaultName}`, L("Save"));
  }

  /** A copy of the whole file in another format; the document keeps its own. */
  async function exportCopy(excel: boolean) {
    if (!doc.status) return;
    const current = await call<SaveFormat>("save_format");
    const title = excel ? L("Export for Excel…") : L("Export…");
    const format = await new Promise<SaveFormat | null>((answer) => (saving = { format: current, title, excel, answer }));
    if (!format) return;
    const extension = format.delimiter === "\t" ? "tsv" : "csv";
    const path = await chooseTarget(`${exportName()}.${extension}`, extension);
    if (!path) return;
    exporting += 1;
    try {
      await call("save_copy", { path, format });
    } catch (error) {
      await report(error);
    } finally {
      exporting -= 1;
    }
  }

  /** The shown rows, in the order shown, as JSON, Markdown, HTML, SQL or an Excel workbook. */
  async function exportAs(kind: ExportKind) {
    const status = doc.status;
    if (!status) return;
    const viewed = status.view.sort.length > 0 || status.view.filters.length > 0;
    const path = await chooseTarget(`${exportName()}.${extensions[kind]}`, extensions[kind], viewed ? L("Only the rows shown are exported, in the order shown.") : undefined);
    if (!path) return;
    exporting += 1;
    try {
      await call("export_file", { path, format: kind, name: exportName() });
    } catch (error) {
      await report(error);
    } finally {
      exporting -= 1;
    }
  }

  /** The selected cells, or all shown, as text for the clipboard; HTML also as HTML. */
  async function copyAs(kind: Exclude<ExportKind, "xlsx">) {
    const selected = grid?.selection;
    if (!selected) return;
    try {
      const text = await call<string>("export_text", { format: kind, range: selected.single ? null : selected.range, name: exportName() });
      await (kind === "html" ? writeClipboardHtml(text, text) : writeClipboard(text));
    } catch (error) {
      await report(error);
    }
  }

  /** Before a window with changes closes: save them, drop them, or keep the window. */
  async function confirmClose(): Promise<boolean> {
    const status = doc.status;
    if (!status?.dirty) return true;
    const answer = await ask(
      L("Do you want to save the changes made to the document “%@”?", name(status.path)),
      L("Your changes will be lost if you don’t save them."),
      [
        { label: L("Don’t Save"), value: "discard" },
        { label: L("Cancel"), value: "cancel", cancel: true },
        { label: L("Save…"), value: "save", default: true },
      ],
    );
    if (answer === "cancel") return false;
    return answer === "discard" || (await save());
  }

  /** An Excel 97–2003 workbook, open for viewing only. */
  const readOnly = () => !!doc.status?.book?.legacy;

  /**
   * Says why a workbook open for viewing only does not change, and offers to export the sheet,
   * which can be changed (CSVDocument.presentReadOnlyNotice).
   */
  async function readOnlyNotice() {
    const answer = await ask(
      L("Excel 97–2003 workbooks open for viewing only"),
      L("CommaEditor cannot save workbooks in this old format. To change the data, export the sheet with File ▸ Export As ▸ Excel Workbook (XLSX) or File ▸ Export… as CSV, and open the copy."),
      [
        { label: L("Export…"), value: "export" },
        { label: L("OK"), value: "ok", default: true, cancel: true },
      ],
    );
    if (answer === "export") await exportCopy(false);
  }

  /**
   * Whether the document can change now; says why not in the status bar. `edits` is false for
   * what only changes how the table shows, which a workbook open for viewing only takes too.
   */
  function changeable(edits = true): boolean {
    const status = doc.status;
    if (!status) return false;
    if (edits && status.book?.legacy) {
      void readOnlyNotice();
      return false;
    }
    if (status.indexing !== null) {
      doc.problem = `${L("The file is still being read.")} ${L("Changes and saving are possible once the whole file is read; the progress is in the status bar.")}`;
      return false;
    }
    if (status.job) {
      doc.problem = `${L("Another change is being made.")} ${L("Wait until it ends or cancel it in the status bar.")}`;
      return false;
    }
    return true;
  }

  async function cellValue(cell: Cell): Promise<string> {
    const loaded = grid?.value(cell);
    if (loaded !== undefined) return loaded;
    const { rows } = await call<{ rows: string[][] }>("rows", { start: cell.row, count: 1 });
    return rows[0]?.[cell.column] ?? "";
  }

  /**
   * Edits a cell: in place, or in a window when its value has line breaks or `window` asks. A
   * cell of a sheet is edited as its formula, or as what was typed into it when it shows that
   * in a number format; a cell a merged area hides, as the area's first cell, as in Excel.
   */
  async function edit(cell: Cell, typed?: string, inWindow = false) {
    if (!grid || !editor || !changeable()) return;
    if (typed !== undefined && !inWindow) {
      // Typing replaces the value: the editor opens at once, so that no key typed next is lost.
      // A cell a merged area hides hands the typing on to the area's first cell.
      editShown = grid.value(cell) ?? "";
      editor.start(cell, editShown, typed);
      if (!doc.status?.isSheet) return;
      const text = await call<{ merged: Cell | null }>("cell_text", cell).catch(() => null);
      const target = editor.target;
      if (text?.merged && target?.row === cell.row && target.column === cell.column) {
        editor.retarget(text.merged);
        grid.select(Selection.at(text.merged));
      }
      return;
    }
    const shown = await cellValue(cell);
    let value = shown;
    if (doc.status?.isSheet) {
      const text = await call<{ formula: string | null; input: string | null; merged: Cell | null }>("cell_text", cell).catch(() => null);
      if (text?.merged) {
        grid.select(Selection.at(text.merged));
        await edit(text.merged, typed, inWindow);
        return;
      }
      value = text?.formula ?? text?.input ?? shown;
    }
    editShown = shown;
    if (inWindow || value.includes("\n") || value.includes("\r")) {
      sheet = { cell, heading: L("Row %ld, column %@", cell.row + 1, grid.title(cell.column)), value };
      return;
    }
    editor.start(cell, value, typed);
  }
  /** The value shown in the cell being edited: typing it again changes nothing. */
  let editShown = "";

  async function write(cell: Cell, value: string | null, move: Move) {
    if (value !== null && value !== editShown) await doc.attempt(() => doc.run("set_cell", { row: cell.row, column: cell.column, value }));
    if (!grid || !move) return;
    const to = {
      row: move === "down" ? cell.row + 1 : cell.row,
      column: move === "right" ? cell.column + 1 : move === "left" ? cell.column - 1 : cell.column,
    };
    const rows = doc.status?.rows ?? 0;
    const columns = doc.status?.columns ?? 0;
    grid.select(Selection.at({ row: Math.min(Math.max(0, to.row), rows - 1), column: Math.min(Math.max(0, to.column), columns - 1) }));
  }

  async function clear() {
    const range = grid?.selection?.range;
    if (range && changeable()) await doc.attempt(() => doc.run("fill", { range, value: "" }));
  }

  async function undo(redo: boolean) {
    const status = doc.status;
    if (!status || !changeable() || !(redo ? status.canRedo : status.canUndo)) return;
    await doc.attempt(() => doc.run(redo ? "redo" : "undo"));
  }

  const sortedOrFiltered = () => (doc.status ? doc.status.view.sort.length > 0 || doc.status.view.filters.length > 0 : false);

  /** Inserts a row above or below the selection, or at the end; not while sorted or filtered. */
  async function insertRow(below: boolean) {
    const status = doc.status;
    if (!status || !grid || sortedOrFiltered() || !changeable()) return;
    const range = grid.selection?.range;
    const at = range ? (below ? range.bottom + 1 : range.top) : status.rows;
    const column = grid.selection?.cursor.column ?? 0;
    if (await doc.attempt(() => doc.run("insert_rows", { at, count: 1 }))) grid.select(Selection.at({ row: at, column }));
  }

  /** Deletes every row the selection touches. */
  async function deleteRows() {
    const range = grid?.selection?.range;
    if (!grid || !range || !changeable()) return;
    const column = grid.selection!.cursor.column;
    if (await doc.attempt(() => doc.run("delete_rows", { top: range.top, bottom: range.bottom }))) {
      const rows = doc.status?.rows ?? 0;
      grid.select(rows > 0 ? Selection.at({ row: Math.min(range.top, rows - 1), column }) : null);
    }
  }

  async function insertColumn(right: boolean) {
    if (!grid || !doc.status || !changeable()) return;
    const cursor = grid.selection?.cursor ?? { row: 0, column: doc.status.columns - 1 };
    const at = cursor.column + (right ? 1 : 0);
    if (await changeColumns(() => doc.run("insert_column", { at }), () => grid!.insertColumn(at))) grid.select(Selection.at({ row: cursor.row, column: at }));
  }

  async function deleteColumn() {
    const cursor = grid?.selection?.cursor;
    if (!grid || !cursor || (doc.status?.columns ?? 0) < 2 || !changeable()) return;
    const at = cursor.column;
    await changeColumns(() => doc.run("delete_column", { at }), () => grid!.deleteColumn(at));
  }

  /** Renames a column, which the header row names; files without one have no names to change. */
  async function renameColumn(column: number) {
    const status = doc.status;
    if (!status?.hasHeader || !changeable()) return;
    const name = await askText(L("Rename Column"), status.header[column] ?? "", L("Rename"));
    if (name !== null && name !== status.header[column]) await doc.attempt(() => doc.run("rename_column", { column, name }));
  }

  /**
   * Runs work that moves rows about, keeping the active row of the file selected (as
   * GridViewController.reselect): it may now show elsewhere, or not at all.
   */
  async function keepingRow(work: () => Promise<unknown>) {
    const cursor = grid?.selection?.cursor;
    const record = cursor ? await call<number | null>("record", { row: cursor.row }) : null;
    await doc.attempt(work);
    if (!grid || !cursor) return;
    const row = record === null ? null : await call<number | null>("visible_index", { row: record });
    grid.select(row === null ? null : Selection.at({ row, column: cursor.column }));
  }

  /**
   * The first row names the columns, or is a row of the table. In a workbook it is CSV mode, the
   * setting of every sheet: the selections kept for the other sheets move with their rows.
   * Frozen rows stay the same rows of the file.
   */
  async function toggleHeader() {
    const status = doc.status;
    if (!status || !changeable(false)) return;
    const frozenTop = (grid?.frozenRows ?? 0) + (status.hasHeader ? 1 : 0);
    const shift = status.hasHeader ? 1 : -1;
    await keepingRow(() => doc.run("set_has_header", { hasHeader: !status.hasHeader }));
    for (const state of sheetStates.values()) {
      if (state.selection) state.selection = state.selection.shifted(shift);
    }
    grid?.freezeRows(Math.max(0, Math.min(frozenTop - (doc.status?.hasHeader ? 1 : 0), (doc.status?.rows ?? 1) - 1)));
    layoutChanged += 1;
  }

  /** Copies the selection as tab-separated text, which spreadsheets paste as cells; or cuts it. */
  async function copy(cut: boolean) {
    const range = grid?.selection?.range;
    if (!range || (cut && !changeable())) return;
    try {
      await writeClipboard(await call<string>("copy", { range }));
      if (cut) await doc.attempt(() => doc.run("fill", { range, value: "" }));
    } catch (error) {
      doc.problem = describe(error).message;
    }
  }

  /** Pastes from the selection's top left, or fills it with a single value; or appends rows. */
  async function paste(text: string) {
    const status = doc.status;
    if (!grid || !status || !text || !changeable()) return;
    const range = grid.selection?.range ?? { top: status.rows, left: 0, bottom: status.rows, right: 0 };
    await doc.attempt(async () => {
      const reply = await doc.runWith<{ status: Status; range: CellRange | null }>("paste", { range, text });
      const written = reply.range;
      if (written) grid!.select(new Selection({ row: written.bottom, column: written.right }, { row: written.top, column: written.left }));
    });
  }

  /** Copies the first row of the selection down it; a single row takes the row above. */
  async function fillDown() {
    const range = grid?.selection?.range;
    if (!range || !changeable()) return;
    const rows = range.top === range.bottom ? { ...range, top: range.top - 1 } : range;
    if (rows.top < 0) return;
    await doc.attempt(() => doc.run("fill_down", { range: rows }));
  }

  // The Data menu (GridViewController+Columns.swift).

  /** The selection, or the active cell's whole column of shown rows. */
  function operationRange(): CellRange | null {
    const selected = grid?.selection;
    const rows = doc.status?.rows ?? 0;
    if (!selected || rows === 0) return null;
    if (!selected.single) return selected.range;
    const column = selected.cursor.column;
    return { top: 0, left: column, bottom: rows - 1, right: column };
  }

  /**
   * Makes a change to many rows, saying what it does while a large table changes in the
   * background; `null` when cancelled or failed, the failure shown.
   */
  async function change(label: string, request: object): Promise<EditResult | null> {
    if (!changeable()) return null;
    editing = label;
    try {
      return await doc.edit(request);
    } catch (error) {
      await report(error);
      return null;
    } finally {
      editing = "";
    }
  }

  function transform(kind: "trim" | "collapseSpaces" | "uppercase" | "lowercase" | "capitalize") {
    const range = operationRange();
    if (range) void change(L("Changing text…"), { transform: { range, transform: kind } });
  }

  async function fillEmpty(fromAbove: boolean) {
    const range = operationRange();
    if (!range || !changeable()) return;
    let value = "";
    if (!fromAbove) {
      const answer = await askForm(L("Fill Empty Cells"), L("Value for the empty cells of the selection:"), { field: "" });
      if (!answer) return;
      value = answer.text;
    }
    await change(L("Filling empty cells…"), { fillEmpty: { range, fromAbove, value } });
  }

  async function splitColumn() {
    const column = grid?.selection?.cursor.column;
    if (column === undefined || !changeable()) return;
    const answer = await askForm(L("Split Column “%@”", grid!.title(column)), L("The parts go to new columns to the right; the column itself stays."), {
      field: "",
      fieldLabel: L("Separator:"),
      placeholder: L("empty: split at spaces"),
      checkbox: L("Regular expression"),
    });
    if (!answer) return;
    // No separator splits at runs of spaces.
    const [separator, regex] = answer.text === "" ? ["\\s+", true] : [answer.text, answer.checked];
    const result = await change(L("Splitting the column…"), { splitColumn: { column, separator, regex } });
    if (result && result.count === 0) await inform(L("Nothing to split"), L("No value in this column contains the separator."));
  }

  async function mergeColumns() {
    const range = grid?.selection?.range;
    if (!range || range.right === range.left || !changeable()) return;
    const columns = Array.from({ length: range.right - range.left + 1 }, (_, index) => range.left + index);
    const titles = columns.map((column) => grid!.title(column)).join(", ");
    const answer = await askForm(L("Merge Columns"), L("Joins %@ into a new column after them; empty values are skipped.", titles), {
      field: "",
      fieldLabel: L("Separator:"),
      placeholder: L("empty: a space"),
    });
    if (answer) await change(L("Merging columns…"), { mergeColumns: { columns, separator: answer.text === "" ? " " : answer.text } });
  }

  async function removeDuplicates() {
    const selected = grid?.selection;
    const total = doc.status?.columns ?? 0;
    if (!selected || !changeable()) return;
    const range = selected.range;
    const width = range.right - range.left + 1;
    // The selected columns are the key, unless they are all of them.
    const columns = width > 0 && width < total ? Array.from({ length: width }, (_, index) => range.left + index) : [];
    const scope = columns.length
      ? L("Rows count as duplicates when they match in %@.", columns.map((column) => grid!.title(column)).join(", "))
      : L("Rows count as duplicates when they match in every column.");
    const answer = await askForm(L("Remove Duplicate Rows"), `${scope} ${L("The first of each set of equal rows stays.")}`, {
      checkbox: L("Ignore case and surrounding spaces"),
    });
    if (!answer) return;
    const result = await change(L("Removing duplicates…"), { removeDuplicates: { columns, normalize: answer.checked } });
    if (!result) return;
    if (result.count === 0) await inform(L("No duplicates found"));
    else await inform(L("Rows removed: %@", count(result.count)), L("Choose Edit ▸ Undo to bring them back."));
  }

  /** Data ▸ Computed Column: a new column after the active one, or new values for it. */
  async function applyFormula(choice: { formula: string; target: "new" | "active"; name: string }, column: number) {
    if (!grid || !doc.status) return;
    const row = grid.selection?.cursor.row ?? 0;
    let report: FormulaReport | null = null;
    if (choice.target === "new") {
      const at = column + 1;
      const before = doc.status.change;
      const old = grid.columnLayout;
      const result = await change(L("Computing the column…"), { addFormulaColumn: { formula: choice.formula, name: choice.name, at } });
      if (!result) return;
      grid.insertColumn(at);
      grid.update();
      layouts.push({ sheet: doc.status.book?.index ?? null, before, after: doc.status.change, old, next: grid.columnLayout });
      grid.select(Selection.at({ row, column: at }));
      report = result.report;
    } else {
      const result = await change(L("Computing the column…"), { fillWithFormula: { formula: choice.formula, column } });
      report = result?.report ?? null;
    }
    if (report && report.errors > 0) {
      let details = L("These cells were left empty. Undo with ⌘Z.");
      if (!mac) details = details.replace("⌘Z", "Ctrl+Z");
      if (report.firstErrorRow !== null && report.firstError) {
        const number = report.firstErrorRow - (doc.status.hasHeader ? 1 : 0) + 1;
        details = `${L("First: row %@, %@.", count(number), describeFormula(report.firstError))} ${details}`;
      }
      await inform(L("The formula failed in %@ of %@ rows.", count(report.errors), count(report.rows)), details);
    }
  }

  // Recipes (GridViewController+Recipes.swift).

  async function loadRecipes() {
    recipes = await call<Recipe[]>("recipes").catch(() => []);
    void installMenu();
  }

  async function toggleRecording() {
    const status = doc.status;
    if (!status || status.indexing !== null) return;
    if (status.recording === null) {
      await doc.attempt(() => doc.run("start_recording"));
      return;
    }
    const { steps } = await doc.runWith<{ status: Status; steps: number }>("stop_recording");
    if (steps === 0) {
      await inform(
        L("Nothing was recorded"),
        L(
          "Recipes record changes to whole columns, columns themselves and the rows a filter selects. Edits of single cells or of chosen rows are not repeated on another file.",
        ),
      );
      return;
    }
    const base = name(status.path).replace(/\.[^.]*$/, "");
    const answer = await askForm(L("Save Recipe"), L("Steps recorded: %@. Name the recipe to apply it to the next file from the Recipes menu.", count(steps)), {
      field: status.path ? base : "",
      fieldLabel: L("Name:"),
    });
    if (!answer) return;
    try {
      await call("save_recorded", { name: answer.text.trim() || L("Untitled Recipe") });
      await loadRecipes();
    } catch (error) {
      await report(error);
    }
  }

  async function applyRecipe(recipe: Recipe) {
    managing = false;
    if (!changeable()) return;
    editing = L("Applying the recipe…");
    try {
      const reply = await doc.runWith<{ status: Status; result: EditResult | null }>("apply_recipe", { file: recipe.file });
      if (!reply.result) {
        const ended = await doc.waitForJob();
        if (ended?.error) throw new CoreFailure(ended.error);
      }
    } catch (error) {
      await report(error);
    } finally {
      editing = "";
    }
  }

  /** The recipe as a pandas script that reads this file and writes a cleaned copy. */
  async function exportPython(recipe: Recipe) {
    const file = name(doc.status?.path ?? null);
    const input = doc.status?.path ? file : "input.csv";
    const dot = input.lastIndexOf(".");
    const output = dot > 0 ? `${input.slice(0, dot)}-clean${input.slice(dot)}` : `${input}-clean.csv`;
    const path = inTauri
      ? await chooseSavePath({ defaultPath: `${recipe.name || "recipe"}.py`, filters: [{ name: "Python", extensions: ["py"] }] })
      : await askText(L("Export for Python…"), `${recipe.name || "recipe"}.py`, L("Save"));
    if (!path) return;
    try {
      await call("recipe_pandas", { file: recipe.file, input, output, delimiter: doc.status?.delimiter ?? ",", path });
    } catch (error) {
      await report(error);
    }
  }

  /** Mends text read in the wrong encoding: the selection, or the whole file with its header. */
  async function fixGarbled() {
    const selected = grid?.selection;
    const range = selected && !selected.single ? selected.range : null;
    const result = await change(L("Fixing garbled text…"), { fixGarbled: { range } });
    if (result && result.count === 0) doc.problem = L("Nothing to fix");
  }

  /** Sorting and filtering wait for indexing and for changes to many rows, but replace each other. */
  function viewable(): boolean {
    const status = doc.status;
    if (!status) return false;
    if (status.indexing !== null || status.job?.kind === "edit") return changeable();
    return true;
  }

  /** Shows the rows as `view` sorts and filters them, keeping the active row of the file. */
  async function setView(view: ViewSpec) {
    if (!viewable()) return;
    filterProblem = "";
    await keepingRow(async () => {
      try {
        const status = await doc.run("set_view", { view });
        if (status.job) await doc.waitForJob();
      } catch (error) {
        const detail = error instanceof CoreFailure ? error.detail : null;
        if (detail && typeof detail === "object" && "invalidPattern" in detail) filterProblem = L("Invalid regular expression");
        else throw error;
      }
    });
  }

  const currentView = (): ViewSpec => doc.status?.view ?? { sort: [], filters: [] };

  function sortBy(keys: SortKey[]) {
    void setView({ ...currentView(), sort: keys });
  }

  /**
   * A click on a column's title sorts by it (GridViewController.didClick): ascending, then
   * descending, then not; with Shift it adds the column to the keys or turns its own.
   */
  function headerClick(column: number, extend: boolean) {
    const keys = currentView().sort.map((key) => ({ ...key }));
    const index = keys.findIndex((key) => key.column === column);
    if (index >= 0 && (extend || keys.length === 1)) {
      if (keys[index].ascending) keys[index].ascending = false;
      else keys.splice(index, 1);
    } else if (extend) {
      keys.push({ column, ascending: true });
    } else {
      keys.splice(0, keys.length, { column, ascending: true });
    }
    sortBy(keys);
  }

  function setFilters(filters: FilterRule[]) {
    void setView({ ...currentView(), filters });
  }

  /** Shows or hides the filter bar; hiding it shows every row again, as in the macOS app. */
  function toggleFilter() {
    if (filtering) {
      filtering = false;
      if (currentView().filters.length > 0) setFilters([]);
      grid?.focus();
    } else {
      filtering = true;
    }
  }

  /** Adds a condition from a cell's menu: only this value, or all but it. */
  async function filterBy(cell: Cell, only: boolean) {
    const value = await cellValue(cell);
    const rule: FilterRule =
      value === ""
        ? { column: cell.column, op: only ? "isEmpty" : "isNotEmpty", value: "", caseSensitive: false }
        : { column: cell.column, op: only ? "equals" : "notEquals", value, caseSensitive: true };
    // The bar applies its conditions, the new one among them.
    filtering = true;
    await tick();
    filterBar?.addRule(rule);
  }

  // The column profile (GridViewController+Profile.swift).

  let profileAsked = 0;
  let profileTimer = 0;

  /** Profiles the active column after a pause; the newest request wins. */
  function refreshProfile() {
    clearTimeout(profileTimer);
    if (panel !== "profile") return;
    profileTimer = window.setTimeout(async () => {
      const column = grid?.selection?.cursor.column ?? 0;
      if (!doc.status || doc.status.columns === 0) return;
      const asked = ++profileAsked;
      profiling = true;
      try {
        const result = await call<ColumnProfile>("profile", { column });
        if (asked !== profileAsked || panel !== "profile") return;
        profile = result;
        profiledColumn = column;
        profiledName = grid?.title(column) ?? "";
        markIssues(column, result);
      } catch {
        if (asked === profileAsked) profile = null;
      } finally {
        if (asked === profileAsked) profiling = false;
      }
    }, 150);
  }

  /** Marks the profiled column's problem cells in the table while the profile shows. */
  function markIssues(column: number, profile: ColumnProfile) {
    grid?.setMarker({
      column,
      fetch: (start, count) => call<number[]>("issue_marks", { start, count, column, kind: profile.kind }),
      describe: (mark) => (mark === 1 ? L("Not %@", kindName(profile.kind).toLowerCase()) : L("Spaces around the value")),
    });
  }

  /** Shows one of the panels at the right of the table, or none (showSidePanel). */
  function showPanel(next: "profile" | "repair" | "card" | null) {
    if (next === panel) return;
    if (panel === "profile") {
      profile = null;
      profiledColumn = null;
      grid?.setMarker(null);
    }
    panel = next;
    if (next === "profile") refreshProfile();
    else if (next === "repair") void repairCursorMoved();
    else if (next === "card") void refreshCard();
    else grid?.focus();
  }

  const toggleCard = () => showPanel(panel === "card" ? null : "card");

  // The record card (GridViewController+Card.swift).

  let cardAsked = 0;

  /** Shows the active row in the card. */
  async function refreshCard() {
    const status = doc.status;
    const cursor = grid?.selection?.cursor;
    if (panel !== "card" || !status) return;
    if (!cursor || status.rows === 0 || status.columns === 0) {
      card = null;
      return;
    }
    const asked = ++cardAsked;
    const { rows } = await call<{ rows: string[][] }>("rows", { start: cursor.row, count: 1 });
    const values = rows[0] ?? [];
    const record = await call<number | null>("record", { row: cursor.row });
    if (asked !== cardAsked) return;
    const number = record === null ? cursor.row + 1 : record - (status.hasHeader && !status.isSheet ? 1 : 0) + 1;
    card = {
      row: cursor.row,
      title: L("Row %@", count(number)),
      position: L("%@ of %@", count(cursor.row + 1), count(status.rows)),
      fields: Array.from({ length: status.columns }, (_, column) => ({ column, name: grid?.title(column) ?? "", value: values[column] ?? "" })),
    };
  }

  /** The previous or next row, in the card and the table. */
  function moveRecord(offset: number) {
    const cursor = grid?.selection?.cursor;
    const rows = doc.status?.rows ?? 0;
    if (!cursor) return;
    const row = cursor.row + offset;
    if (row >= 0 && row < rows) grid?.select(Selection.at({ row, column: cursor.column }));
  }

  const toggleProfile = () => showPanel(panel === "profile" ? null : "profile");
  const toggleRepair = () => showPanel(panel === "repair" ? null : "repair");

  // File repair (GridViewController+Repair.swift).

  const LISTED_PROBLEMS = 5000;
  let repairTimer = 0;
  let repairAsked = 0;
  /** After a fix, the problem at this place in the list is selected next. */
  let pendingIndex: number | null = null;

  /** Looks for broken rows a moment after the last change; a file being read waits. */
  function scheduleProblemCheck(delay = 300) {
    clearTimeout(repairTimer);
    if (!doc.status || doc.status.indexing !== null) return;
    repairTimer = window.setTimeout(() => void checkProblems(), delay);
  }

  async function checkProblems() {
    const asked = ++repairAsked;
    repairLoading = true;
    try {
      const report = await call<RepairReport>("find_problems", { limit: LISTED_PROBLEMS });
      if (asked !== repairAsked) return;
      const first = repair === null;
      repair = report;
      repairLoading = false;
      grid?.setRowMarker(report.problems.length > 0 ? (start, count) => call<number[]>("problem_marks", { start, count }) : null);
      garbledExample = report.garbledSample > 0 ? await call<[string, string] | null>("garbled_example") : null;
      const total = report.counts.reduce((sum, item) => sum + item.count, 0);
      // A file that opens with broken rows or garbled text shows them right away.
      if (first && (total > 0 || report.garbledSample > 0) && panel === null) showPanel("repair");
      reselectProblem();
    } catch {
      if (asked === repairAsked) repairLoading = false;
    }
  }

  /** After a check, keeps the selected problem, or after a fix moves on to the next one. */
  function reselectProblem() {
    const problems = repair?.problems ?? [];
    const kept = problems.find((problem) => problem.row === repairRow);
    if (kept) void selectProblem(kept, false);
    else if (pendingIndex !== null && problems.length > 0) void selectProblem(problems[Math.min(pendingIndex, problems.length - 1)], true);
    else {
      repairRow = null;
      repairOptions = [];
      void repairCursorMoved();
    }
    pendingIndex = null;
  }

  /** Shows a problem's fixes, and with `moveCursor` its row in the table. */
  async function selectProblem(problem: Problem, moveCursor: boolean) {
    repairRow = problem.row;
    repairRaw = await call<string>("raw_text", { row: problem.row, maxChars: 600 }).catch(() => "");
    const options = await call<RepairOption[]>("repair_options", { row: problem.row }).catch(() => []);
    if (repairRow === problem.row) repairOptions = options;
    if (moveCursor) {
      const visible = await call<number | null>("visible_index", { row: problem.row });
      if (visible !== null) grid?.select(Selection.at({ row: visible, column: grid.selection?.cursor.column ?? 0 }));
    }
  }

  /** The problem of the active row becomes the one selected in the panel. */
  async function repairCursorMoved() {
    const cursor = grid?.selection?.cursor;
    if (panel !== "repair" || !cursor || !repair) return;
    const record = await call<number | null>("record", { row: cursor.row });
    const problem = repair.problems.find((item) => item.row === record);
    if (problem && problem.row !== repairRow) await selectProblem(problem, false);
  }

  async function applyFix(row: number, fix: RepairFix) {
    if (!changeable()) return;
    pendingIndex = repair?.problems.findIndex((problem) => problem.row === row) ?? null;
    repairRow = null;
    await doc.attempt(() => doc.run("apply_fix", { row, fix }));
  }

  async function fixAll(kind: ProblemKind) {
    if (!changeable()) return;
    pendingIndex = 0;
    repairRow = null;
    await doc.attempt(() => doc.runWith("fix_all", { kind }));
  }

  const repairRowLabel = (row: number) => {
    const offset = doc.status?.hasHeader ? 1 : 0;
    return row < offset ? L("Header") : L("Row %@", count(row - offset + 1));
  };

  /** The status bar's button to the repair panel: broken rows, or garbled text. */
  const problemsLabel = $derived.by(() => {
    if (!repair) return "";
    const total = repair.counts.reduce((sum, item) => sum + item.count, 0);
    if (total > 0) return L("Problems: %@", count(total));
    return repair.garbledSample > 0 ? L("Garbled text") : "";
  });

  /** Selects the next cell after the active row with the problem. */
  async function showIssue(issue: "mismatch" | "padding") {
    if (profiledColumn === null || !profile) return;
    const after = grid?.selection?.cursor.row ?? null;
    const row = await call<number | null>("find_issue", { column: profiledColumn, issue, kind: profile.kind, after });
    if (row !== null) grid?.select(Selection.at({ row, column: profiledColumn }));
  }

  function trimProfiled() {
    const rows = doc.status?.rows ?? 0;
    if (profiledColumn === null || rows === 0) return;
    const range = { top: 0, left: profiledColumn, bottom: rows - 1, right: profiledColumn };
    void change(L("Changing text…"), { transform: { range, transform: "trim" } });
  }

  async function filterValue(value: string) {
    if (profiledColumn === null) return;
    const rule: FilterRule = { column: profiledColumn, op: value === "" ? "isEmpty" : "equals", value, caseSensitive: true };
    filtering = true;
    await tick();
    filterBar?.addRule(rule);
  }

  // The table's view (GridViewController+Tables.swift and +FrozenRows.swift): kept in the
  // window only, as in the macOS app.

  /** The selected columns, or the active one. */
  function selectedColumns(): number[] {
    const range = grid?.selection?.range;
    if (!range) return [];
    return Array.from({ length: range.right - range.left + 1 }, (_, index) => range.left + index);
  }

  /** Fits the selected columns, or all of them with a single cell selected. */
  function fitColumns() {
    if (!grid || !doc.status) return;
    const selected = grid.selection;
    const columns = selected && !selected.single ? selectedColumns() : Array.from({ length: doc.status.columns }, (_, column) => column);
    void grid.fitColumns(columns);
  }

  function hideColumns() {
    if (!grid || !doc.status) return;
    const columns = selectedColumns();
    if (doc.status.columns - grid.hiddenCount - columns.filter((column) => !grid!.isHidden(column)).length < 1) return;
    grid.hideColumns(columns);
    // The cursor goes to the next shown column, to the right or else to the left.
    const cursor = grid.selection!.cursor;
    const right = Math.max(...columns) + 1;
    const next = grid.shownColumn(Math.min(right, doc.status.columns - 1), right < doc.status.columns ? 1 : -1);
    if (next !== null) grid.select(Selection.at({ row: cursor.row, column: next }));
    layoutChanged += 1;
  }

  function showAllColumns() {
    grid?.showAllColumns();
    layoutChanged += 1;
  }

  /** Bumps when the view's layout changes, for the menus and the status bar. */
  let layoutChanged = $state(0);

  function freezeColumns(through: number | null) {
    grid?.freezeColumns(through === null ? 0 : through + 1);
    layoutChanged += 1;
  }

  function freezeRows(through: number | null) {
    grid?.freezeRows(through === null ? 0 : through + 1);
    layoutChanged += 1;
  }

  function setLines(lines: number) {
    grid?.setLinesPerRow(lines);
    layoutChanged += 1;
  }

  function toggleInvisibles() {
    grid?.setInvisibles(!grid.showsInvisibles);
    layoutChanged += 1;
  }

  /** Rows can move while not sorted, filtered or not (GridViewController.canMoveRows). */
  const canMoveRows = () => idle() && (doc.status?.view.sort.length ?? 1) === 0;

  /** Moves the selected rows before visible row `before`, keeping them selected. */
  async function moveRows(before: number) {
    const selected = grid?.selection;
    if (!grid || !selected || !canMoveRows()) return;
    const { top, bottom } = selected.range;
    if (before >= top && before <= bottom + 1) return;
    const moved = await doc.attempt(() => doc.runWith("move_rows", { top, bottom, to: before }));
    if (!moved) return;
    const start = before > bottom ? before - (bottom - top + 1) : before;
    const shift = start - top;
    grid.select(
      new Selection({ row: selected.anchor.row + shift, column: selected.anchor.column }, { row: selected.cursor.row + shift, column: selected.cursor.column }),
    );
  }

  /** Moves a column; the core shows every row again, as it reorders the fields. */
  async function moveColumn(from: number, to: number) {
    if (!grid || !idle() || to < 0 || to >= (doc.status?.columns ?? 0)) return;
    const row = grid.selection?.cursor.row ?? 0;
    const moved = await doc.attempt(async () => {
      const reply = await doc.runWith<{ status: Status; moved: boolean }>("move_column", { from, to });
      if (reply.moved) grid!.moveColumn(from, to);
    });
    grid.update();
    if (moved) grid.select(Selection.at({ row, column: to }));
  }

  /** Opens the filter bar with a condition for the column whose filter icon was clicked. */
  async function filterColumn(column: number) {
    filtering = true;
    await tick();
    await filterBar?.beginRule(column);
  }

  async function reorderRows() {
    if (changeable()) await keepingRow(() => doc.run("apply_sort"));
  }


  /** The menus of a cell and of a column's title (GridViewController+View.swift). */
  function openMenu(target: { area: "cell"; cell: Cell } | { area: "header"; column: number }, x: number, y: number) {
    const status = doc.status;
    if (!status) return;
    const busy = status.indexing !== null || status.job !== null;
    if (target.area === "header") {
      const column = target.column;
      menu = {
        x,
        y,
        items: [
          { label: L("Sort Ascending"), run: () => sortBy([{ column, ascending: true }]) },
          { label: L("Sort Descending"), run: () => sortBy([{ column, ascending: false }]) },
          { label: L("Clear Sort"), disabled: status.view.sort.length === 0, run: () => sortBy([]) },
          {
            label: L("Reorder Rows in File"),
            disabled: busy || status.view.sort.length === 0 || status.view.filters.length > 0,
            run: () => void reorderRows(),
          },
          null,
          { label: L("Rename Column…"), disabled: busy || !status.hasHeader, run: () => void renameColumn(column) },
          null,
          { label: L("Insert Column Left"), shortcut: shortcutOf("insertColumnLeft"), disabled: busy, run: () => void insertColumn(false) },
          { label: L("Insert Column Right"), shortcut: shortcutOf("insertColumnRight"), disabled: busy, run: () => void insertColumn(true) },
          { label: L("Delete Column"), shortcut: shortcutOf("deleteColumn"), disabled: busy || status.columns < 2, run: () => void deleteColumn() },
          null,
          menuItem("fitColumns"),
          menuItem("hideColumn"),
          menuItem("showAllColumns"),
          (grid?.frozenColumns ?? 0) > 0 ? menuItem("unfreezeColumns") : menuItem("freezeColumns"),
          null,
          { label: L("First Row Is Header"), checked: status.hasHeader, disabled: busy, run: () => void toggleHeader() },
        ],
      };
      return;
    }
    const cell = target.cell;
    const view = sortedOrFiltered();
    const shown = grid?.value(cell);
    const short = shown && shown.length > 30 ? `${shown.slice(0, 30)}…` : (shown ?? "");
    menu = {
      x,
      y,
      items: [
        { label: short === "" ? L("Show Only Empty") : L("Show Only “%@”", short), run: () => void filterBy(cell, true) },
        { label: short === "" ? L("Hide Empty") : L("Hide “%@”", short), run: () => void filterBy(cell, false) },
        null,
        { label: L("Edit Cell"), disabled: busy, run: () => void edit(cell) },
        { label: L("Edit Cell in Window…"), shortcut: shortcutOf("editInWindow"), disabled: busy, run: () => void edit(cell, undefined, true) },
        null,
        { label: L("Cut"), shortcut: shortcutLabel({ key: "x", command: true }), disabled: busy, run: () => void copy(true) },
        { label: L("Copy"), shortcut: shortcutLabel({ key: "c", command: true }), run: () => void copy(false) },
        { label: L("Paste"), shortcut: shortcutLabel({ key: "v", command: true }), disabled: busy, run: () => void readClipboard().then(paste) },
        { label: L("Fill Down"), shortcut: shortcutOf("fillDown"), disabled: busy, run: () => void fillDown() },
        null,
        { label: L("Insert Row Above"), shortcut: shortcutOf("insertRowAbove"), disabled: busy || view, run: () => void insertRow(false) },
        { label: L("Insert Row Below"), shortcut: shortcutOf("insertRowBelow"), disabled: busy || view, run: () => void insertRow(true) },
        { label: L("Delete Rows"), shortcut: shortcutOf("deleteRows"), disabled: busy, run: () => void deleteRows() },
        null,
        cell.row < (grid?.frozenRows ?? 0) ? menuItem("unfreezeRows") : menuItem("freezeRows"),
      ],
    };
  }

  const idle = () => !!doc.status && doc.status.indexing === null && !doc.status.job;
  const hasSelection = () => !!grid?.selection;
  /** The text field with the focus, for commands that act on its text there. */
  const field = () => (fieldFocused() ? (document.activeElement as HTMLInputElement | HTMLTextAreaElement) : null);

  function openFind(replacing: boolean) {
    if (!doc.open) return;
    if (finding && finding.replacing === replacing) findBar?.focus();
    else finding = { replacing, selection: grid?.selection?.range ?? null };
  }

  /** Every command of the menus and the keyboard (MainMenu.swift). */
  function commandList(): Command[] {
    const status = () => doc.status;
    const list: Command[] = [
      { id: "new", label: L("New"), shortcut: { key: "n", command: true }, inFields: true, run: () => void newDocument() },
      { id: "open", label: L("Open…"), shortcut: { key: "o", command: true }, inFields: true, run: () => void choose() },
      { id: "clearRecent", label: L("Clear Menu"), enabled: () => recentFiles().length > 0, run: () => (clearRecent(), void installMenu()) },
      {
        id: "close",
        label: L("Close"),
        shortcut: { key: "w", command: true },
        inFields: true,
        enabled: () => inTauri,
        run: () => void getCurrentWindow().close(),
      },
      { id: "save", label: L("Save…"), shortcut: { key: "s", command: true }, inFields: true, enabled: () => doc.open, run: () => void save() },
      {
        id: "saveAs",
        label: L("Save As…"),
        shortcut: { key: "s", command: true, shift: true },
        inFields: true,
        enabled: () => doc.open,
        run: () => void saveAs(),
      },
      { id: "exportCopy", label: L("Export…"), enabled: () => doc.open, run: () => void exportCopy(false) },
      {
        id: "exportExcel",
        label: L("Export for Excel…"),
        shortcut: { key: "e", command: true, shift: true },
        inFields: true,
        enabled: () => doc.open,
        run: () => void exportCopy(true),
      },
      ...(
        [
          ["json", L("JSON…")],
          ["markdown", L("Markdown…")],
          ["html", L("HTML…")],
          ["sql", L("SQL…")],
          ["xlsx", L("Excel Workbook (XLSX)…")],
        ] as const
      ).map(([kind, label]): Command => ({ id: `exportAs:${kind}`, label, enabled: () => (doc.status?.columns ?? 0) > 0, run: () => void exportAs(kind) })),
      ...(
        [
          ["markdown", L("Markdown")],
          ["json", L("JSON")],
          ["html", L("HTML")],
          ["sql", L("SQL")],
        ] as const
      ).map(([kind, label]): Command => ({ id: `copyAs:${kind}`, label, enabled: hasSelection, run: () => void copyAs(kind) })),
      {
        id: "compare",
        label: L("Compare with File…"),
        shortcut: { key: "d", command: true, alt: true },
        inFields: true,
        enabled: () => !!doc.status && doc.status.indexing === null,
        run: () => void compareWithFile(),
      },
      {
        id: "checkEncoding",
        label: L("Check Encoding and Delimiter…"),
        enabled: () => !!status()?.path && !status()?.isSheet,
        run: () => (checking = { uncertain: false }),
      },
      {
        id: "reopenEncoding:auto",
        label: L("Detect Automatically"),
        enabled: () => !!status()?.path && !status()?.isSheet,
        checked: () => reading.encoding === null,
        run: () => void reopen(null, reading.delimiter),
      },
      ...encodings.map(
        (encoding): Command => ({
          id: `reopenEncoding:${encoding}`,
          label: encodingName(encoding),
          enabled: () => !!status()?.path && !status()?.isSheet,
          checked: () => reading.encoding === encoding,
          run: () => void reopen(encoding, reading.delimiter),
        }),
      ),
      {
        id: "reopenDelimiter:auto",
        label: L("Detect Automatically"),
        enabled: () => !!status()?.path && !status()?.isSheet,
        checked: () => reading.delimiter === null,
        run: () => void reopen(reading.encoding, null),
      },
      ...delimiters.map(
        (delimiter): Command => ({
          id: `reopenDelimiter:${delimiter}`,
          label: delimiterName(delimiter),
          enabled: () => !!status()?.path && !status()?.isSheet,
          checked: () => reading.delimiter === delimiter,
          run: () => void reopen(reading.encoding, delimiter),
        }),
      ),
      {
        id: "quit",
        label: L("Quit %@", appName),
        shortcut: { key: "q", command: true },
        inFields: true,
        enabled: () => inTauri,
        run: () => void quit(),
      },
      {
        id: "undo",
        label: L("Undo"),
        shortcut: { key: "z", command: true },
        inFields: true,
        enabled: () => !!field() || (idle() && !!status()?.canUndo),
        run: () => (field() ? document.execCommand("undo") : void undo(false)),
      },
      {
        id: "redo",
        label: L("Redo"),
        shortcut: { key: "z", command: true, shift: true },
        inFields: true,
        enabled: () => !!field() || (idle() && !!status()?.canRedo),
        run: () => (field() ? document.execCommand("redo") : void undo(true)),
      },
      { id: "clear", label: L("Delete"), enabled: () => idle() && hasSelection(), run: () => void clear() },
      {
        id: "selectAll",
        label: L("Select All"),
        shortcut: { key: "a", command: true },
        inFields: true,
        enabled: () => !!field() || doc.open,
        run: () => {
          const text = field();
          if (text) text.select();
          else if (grid && doc.status && doc.status.rows > 0)
            grid.select(new Selection({ row: doc.status.rows - 1, column: doc.status.columns - 1 }, { row: 0, column: 0 }), false);
        },
      },
      { id: "find", label: L("Find…"), shortcut: { key: "f", command: true }, inFields: true, enabled: () => doc.open, run: () => openFind(finding?.replacing ?? false) },
      {
        id: "findReplace",
        label: L("Find and Replace…"),
        shortcut: { key: "f", command: true, alt: true },
        inFields: true,
        enabled: () => doc.open,
        run: () => openFind(true),
      },
      {
        id: "findNext",
        label: L("Find Next"),
        shortcut: { key: "g", command: true },
        inFields: true,
        enabled: () => !!finding,
        run: () => void findBar?.next(true),
      },
      {
        id: "findPrevious",
        label: L("Find Previous"),
        shortcut: { key: "g", command: true, shift: true },
        inFields: true,
        enabled: () => !!finding,
        run: () => void findBar?.next(false),
      },
      { id: "editCell", label: L("Edit Cell"), enabled: () => idle() && hasSelection(), run: () => void edit(grid!.selection!.cursor) },
      {
        id: "editInWindow",
        label: L("Edit Cell in Window…"),
        shortcut: { key: "e", command: true, alt: true },
        enabled: () => idle() && hasSelection(),
        run: () => void edit(grid!.selection!.cursor, undefined, true),
      },
      { id: "fillDown", label: L("Fill Down"), shortcut: { key: "d", command: true }, enabled: () => idle() && hasSelection(), run: () => void fillDown() },
      {
        id: "insertRowAbove",
        label: L("Insert Row Above"),
        shortcut: { key: "ArrowUp", command: true, alt: true },
        enabled: () => idle() && !sortedOrFiltered(),
        run: () => void insertRow(false),
      },
      {
        id: "insertRowBelow",
        label: L("Insert Row Below"),
        shortcut: { key: "ArrowDown", command: true, alt: true },
        enabled: () => idle() && !sortedOrFiltered(),
        run: () => void insertRow(true),
      },
      {
        id: "deleteRows",
        label: L("Delete Rows"),
        shortcut: { key: "Backspace", command: true },
        enabled: () => idle() && hasSelection(),
        run: () => void deleteRows(),
      },
      {
        id: "insertColumnLeft",
        label: L("Insert Column Left"),
        shortcut: { key: "ArrowLeft", command: true, alt: true },
        enabled: idle,
        run: () => void insertColumn(false),
      },
      {
        id: "insertColumnRight",
        label: L("Insert Column Right"),
        shortcut: { key: "ArrowRight", command: true, alt: true },
        enabled: idle,
        run: () => void insertColumn(true),
      },
      {
        id: "deleteColumn",
        label: L("Delete Column"),
        shortcut: { key: "Backspace", command: true, alt: true },
        enabled: () => idle() && hasSelection() && (status()?.columns ?? 0) > 1,
        run: () => void deleteColumn(),
      },
      {
        id: "renameColumn",
        label: L("Rename Column…"),
        enabled: () => idle() && hasSelection() && !!status()?.hasHeader,
        run: () => void renameColumn(grid!.selection!.cursor.column),
      },
      {
        id: "sortAscending",
        label: L("Sort Ascending"),
        enabled: hasSelection,
        run: () => sortBy([{ column: grid!.selection!.cursor.column, ascending: true }]),
      },
      {
        id: "sortDescending",
        label: L("Sort Descending"),
        enabled: hasSelection,
        run: () => sortBy([{ column: grid!.selection!.cursor.column, ascending: false }]),
      },
      { id: "clearSort", label: L("Clear Sort"), enabled: () => (status()?.view.sort.length ?? 0) > 0, run: () => sortBy([]) },
      {
        id: "reorderRows",
        label: L("Reorder Rows in File"),
        enabled: () => idle() && (status()?.view.sort.length ?? 0) > 0 && (status()?.view.filters.length ?? 0) === 0,
        run: () => void reorderRows(),
      },
      {
        id: "toggleFilter",
        label: () => (filtering ? L("Hide Filter") : L("Show Filter")),
        shortcut: { key: "l", command: true, shift: true },
        inFields: true,
        enabled: () => doc.open,
        run: toggleFilter,
      },
      {
        id: "clearFilter",
        label: L("Clear Filter"),
        enabled: () => (status()?.view.filters.length ?? 0) > 0,
        run: () => {
          filtering = false;
          setFilters([]);
        },
      },
      {
        id: "toggleHeader",
        label: L("First Row Is Header"),
        checked: () => !!status()?.hasHeader,
        enabled: idle,
        run: () => void toggleHeader(),
      },
      ...(
        [
          ["trim", L("Trim Spaces")],
          ["collapseSpaces", L("Remove Extra Spaces")],
          ["uppercase", L("UPPERCASE")],
          ["lowercase", L("lowercase")],
          ["capitalize", L("Capitalize Each Word")],
        ] as const
      ).map(([kind, label]): Command => ({ id: `text:${kind}`, label, enabled: () => idle() && hasSelection(), run: () => transform(kind) })),
      { id: "fillFromAbove", label: L("Fill Empty Cells from Above"), enabled: () => idle() && hasSelection(), run: () => void fillEmpty(true) },
      { id: "fillEmpty", label: L("Fill Empty Cells…"), enabled: () => idle() && hasSelection(), run: () => void fillEmpty(false) },
      { id: "splitColumn", label: L("Split Column…"), enabled: () => idle() && hasSelection(), run: () => void splitColumn() },
      {
        id: "mergeColumns",
        label: L("Merge Columns…"),
        enabled: () => idle() && !!grid?.selection && grid.selection.columnCount > 1,
        run: () => void mergeColumns(),
      },
      {
        id: "computedColumn",
        label: L("Computed Column…"),
        shortcut: { key: "=", command: true, alt: true },
        enabled: () => idle() && (doc.status?.columns ?? 0) > 0,
        run: () => {
          const column = grid?.selection?.cursor.column ?? 0;
          const columns = Array.from({ length: doc.status?.columns ?? 0 }, (_, index) => grid?.title(index) ?? "");
          computing = { column, name: columns[column] ?? "", columns };
        },
      },
      { id: "fixGarbled", label: L("Fix Garbled Text"), enabled: () => idle(), run: () => void fixGarbled() },
      { id: "removeDuplicates", label: L("Remove Duplicate Rows…"), enabled: () => idle() && hasSelection(), run: () => void removeDuplicates() },
      {
        id: "toggleProfile",
        label: () => (panel === "profile" ? L("Hide Column Profile") : L("Show Column Profile")),
        shortcut: { key: "p", command: true, alt: true },
        inFields: true,
        enabled: () => (doc.status?.columns ?? 0) > 0,
        run: toggleProfile,
      },
      {
        id: "toggleRepair",
        label: () => (panel === "repair" ? L("Hide File Repair") : L("Show File Repair")),
        shortcut: { key: "r", command: true, alt: true },
        inFields: true,
        enabled: () => doc.open,
        run: toggleRepair,
      },
      {
        id: "toggleCard",
        label: () => (panel === "card" ? L("Hide Record Card") : L("Show Record Card")),
        shortcut: { key: "i", command: true, alt: true },
        inFields: true,
        enabled: () => (doc.status?.columns ?? 0) > 0,
        run: toggleCard,
      },
      {
        id: "previousRecord",
        label: L("Previous Record"),
        shortcut: { key: "[", command: true },
        enabled: () => (grid?.selection?.cursor.row ?? 0) > 0,
        run: () => moveRecord(-1),
      },
      {
        id: "nextRecord",
        label: L("Next Record"),
        shortcut: { key: "]", command: true },
        enabled: () => !!grid?.selection && grid.selection.cursor.row < (doc.status?.rows ?? 0) - 1,
        run: () => moveRecord(1),
      },
      {
        id: "previousSheet",
        label: L("Previous Sheet"),
        // ⌃PageUp on a Mac, Ctrl+PageUp elsewhere, as in Excel.
        shortcut: mac ? { key: "PageUp", control: true } : { key: "PageUp", command: true },
        inFields: true,
        enabled: () => neighbourSheet(-1) !== null,
        run: () => void chooseSheet(neighbourSheet(-1)!),
      },
      {
        id: "nextSheet",
        label: L("Next Sheet"),
        shortcut: mac ? { key: "PageDown", control: true } : { key: "PageDown", command: true },
        inFields: true,
        enabled: () => neighbourSheet(1) !== null,
        run: () => void chooseSheet(neighbourSheet(1)!),
      },
      {
        id: "fitColumns",
        label: L("Fit Column Width"),
        shortcut: { key: "0", command: true, alt: true },
        enabled: () => (doc.status?.columns ?? 0) > 0,
        run: fitColumns,
      },
      {
        id: "hideColumn",
        label: L("Hide Column"),
        enabled: () => {
          void layoutChanged;
          return !!grid?.selection && (doc.status?.columns ?? 0) - (grid?.hiddenCount ?? 0) > (grid?.selection?.columnCount ?? 1);
        },
        run: hideColumns,
      },
      {
        id: "showAllColumns",
        label: () => {
          void layoutChanged;
          const hidden = grid?.hiddenCount ?? 0;
          return hidden > 0 ? L("Show All Columns (%@ hidden)", count(hidden)) : L("Show All Columns");
        },
        enabled: () => {
          void layoutChanged;
          return (grid?.hiddenCount ?? 0) > 0;
        },
        run: showAllColumns,
      },
      {
        id: "freezeColumns",
        label: L("Freeze Columns Through This One"),
        enabled: () => !!grid?.selection && (doc.status?.columns ?? 0) > 1,
        run: () => freezeColumns(grid!.selection!.cursor.column),
      },
      {
        id: "unfreezeColumns",
        label: L("Unfreeze Columns"),
        enabled: () => {
          void layoutChanged;
          return (grid?.frozenColumns ?? 0) > 0;
        },
        run: () => freezeColumns(null),
      },
      {
        id: "freezeRows",
        label: L("Freeze Rows Through This One"),
        enabled: () => !!grid?.selection && (doc.status?.rows ?? 0) > 1,
        run: () => freezeRows(grid!.selection!.cursor.row),
      },
      {
        id: "unfreezeRows",
        label: L("Unfreeze Rows"),
        enabled: () => {
          void layoutChanged;
          return (grid?.frozenRows ?? 0) > 0;
        },
        run: () => freezeRows(null),
      },
      ...(
        [
          [1, L("One Line")],
          [2, L("Up to 2 Lines")],
          [3, L("Up to 3 Lines")],
          [5, L("Up to 5 Lines")],
        ] as const
      ).map(
        ([lines, label]): Command => ({
          id: `lines:${lines}`,
          label,
          checked: () => {
            void layoutChanged;
            return grid?.linesPerRow === lines;
          },
          run: () => setLines(lines),
        }),
      ),
      {
        id: "toggleInvisibles",
        label: L("Show Invisible Characters"),
        shortcut: { key: "i", command: true, shift: true },
        inFields: true,
        checked: () => {
          void layoutChanged;
          return !!grid?.showsInvisibles;
        },
        run: toggleInvisibles,
      },
      {
        id: "moveRowsUp",
        label: L("Move Rows Up"),
        shortcut: { key: "ArrowUp", command: true, control: true },
        enabled: () => canMoveRows() && (grid?.selection?.range.top ?? 0) > 0,
        run: () => void moveRows(grid!.selection!.range.top - 1),
      },
      {
        id: "moveRowsDown",
        label: L("Move Rows Down"),
        shortcut: { key: "ArrowDown", command: true, control: true },
        enabled: () => canMoveRows() && !!grid?.selection && grid.selection.range.bottom < (doc.status?.rows ?? 0) - 1,
        run: () => void moveRows(grid!.selection!.range.bottom + 2),
      },
      {
        id: "moveColumnLeft",
        label: L("Move Column Left"),
        shortcut: { key: "ArrowLeft", command: true, control: true },
        enabled: () => idle() && (grid?.selection?.cursor.column ?? 0) > 0,
        run: () => void moveColumn(grid!.selection!.cursor.column, grid!.selection!.cursor.column - 1),
      },
      {
        id: "moveColumnRight",
        label: L("Move Column Right"),
        shortcut: { key: "ArrowRight", command: true, control: true },
        enabled: () => idle() && !!grid?.selection && grid.selection.cursor.column < (doc.status?.columns ?? 0) - 1,
        run: () => void moveColumn(grid!.selection!.cursor.column, grid!.selection!.cursor.column + 1),
      },
      {
        id: "recording",
        label: () => (doc.status?.recording !== null && doc.status?.recording !== undefined ? L("Stop Recording…") : L("Start Recording")),
        enabled: () => !!doc.status && doc.status.indexing === null,
        run: () => void toggleRecording(),
      },
      { id: "noRecipes", label: L("No Saved Recipes"), enabled: () => false, run: () => {} },
      {
        id: "manageRecipes",
        label: L("Manage Recipes…"),
        run: () => void loadRecipes().then(() => (managing = true)),
      },
      ...recipes.map(
        (recipe): Command => ({
          id: `recipe:${recipe.file}`,
          label: recipe.name || L("Untitled Recipe"),
          enabled: idle,
          run: () => void applyRecipe(recipe),
        }),
      ),
      {
        id: "bench",
        label: "Scroll Bench",
        shortcut: { key: "b", command: true, shift: true, alt: true },
        enabled: () => doc.open,
        run: () => void bench().then((text) => (benchText = text)),
      },
    ];
    // A workbook open for viewing only takes no changes; a sheet's rows and columns change as
    // its workbook lets them (GridViewController.validateMenuItem).
    for (const command of list) {
      const enabled = command.enabled;
      const editing = EDITING.has(command.id) || command.id.startsWith("text:") || command.id.startsWith("recipe:");
      command.enabled = () => (!editing || !readOnly()) && sheetAllows(command.id) && (enabled?.() ?? true);
    }
    // Windows and Linux redo with Ctrl-Y too.
    if (!mac) list.push({ ...list.find((command) => command.id === "redo")!, id: "redoY", shortcut: { key: "y", command: true } });
    for (const [index, path] of recentFiles().entries()) {
      list.push({ id: `recent:${index}`, label: `${name(path)} — ${path.slice(0, -name(path).length - 1)}`, run: () => void openPaths([path]) });
    }
    return list;
  }

  /** Commands that change the file, which a document open for viewing only does not take. */
  const EDITING = new Set([
    "save",
    "saveAs",
    "clear",
    "editCell",
    "editInWindow",
    "fillDown",
    "insertRowAbove",
    "insertRowBelow",
    "deleteRows",
    "insertColumnLeft",
    "insertColumnRight",
    "deleteColumn",
    "renameColumn",
    "reorderRows",
    "fillFromAbove",
    "fillEmpty",
    "splitColumn",
    "mergeColumns",
    "computedColumn",
    "fixGarbled",
    "removeDuplicates",
    "moveRowsUp",
    "moveRowsDown",
    "moveColumnLeft",
    "moveColumnRight",
    "recording",
  ]);

  /**
   * Whether a sheet takes a command: rows and columns move with the references to them, unless
   * tables, drawings, comments, charts or pivot tables hold them where they are; then only
   * adding at the end is left.
   */
  function sheetAllows(id: string): boolean {
    const status = doc.status;
    const book = status?.book;
    if (!status?.isSheet || !book) return true;
    const locked = book.structureLocked;
    const cursor = grid?.selection?.cursor;
    switch (id) {
      case "insertColumnRight":
        return !locked || cursor?.column === status.columns - 1;
      case "insertColumnLeft":
      case "deleteColumn":
      case "splitColumn":
      case "mergeColumns":
      case "moveColumnLeft":
      case "moveColumnRight":
        return !locked;
      case "insertRowBelow":
        return !(locked && book.rowsFixed) || cursor?.row === status.rows - 1;
      case "insertRowAbove":
      case "deleteRows":
        return !(locked && book.rowsFixed);
      case "moveRowsUp":
      case "moveRowsDown":
      case "reorderRows":
        return !locked || !book.rowsFixed;
      case "removeDuplicates":
        return !book.rowsFixed;
      default:
        return true;
    }
  }

  /** The visible table before or after the shown sheet (WorkbookDocument.neighbourSheet). */
  function neighbourSheet(step: 1 | -1): number | null {
    const book = doc.status?.book;
    if (!book) return null;
    for (let index = book.index + step; index >= 0 && index < book.sheets.length; index += step) {
      const sheet = book.sheets[index];
      if (sheet.isTable && sheet.visibility === "visible") return index;
    }
    return null;
  }

  let commands = new Commands(commandList());

  /** The menu bar as MainMenu.swift lays it out, for the commands this app has so far. */
  function menuEntries(): { title: string; entries: Entry[] }[] {
    const recent: Entry[] = recentFiles().map((_, index) => `recent:${index}`);
    const file: Entry[] = [
      "new",
      "open",
      { title: L("Open Recent"), entries: [...recent, ...(recent.length ? ["-"] : []), "clearRecent"] },
      "-",
      "close",
      "save",
      "saveAs",
      "exportCopy",
      "exportExcel",
      { title: L("Export As"), entries: ["exportAs:json", "exportAs:markdown", "exportAs:html", "exportAs:sql", "exportAs:xlsx"] },
      "-",
      { title: L("Reopen with Encoding"), entries: ["reopenEncoding:auto", "-", ...encodings.map((encoding) => `reopenEncoding:${encoding}`)] },
      { title: L("Reopen with Delimiter"), entries: ["reopenDelimiter:auto", "-", ...delimiters.map((delimiter) => `reopenDelimiter:${delimiter}`)] },
      "checkEncoding",
      "-",
      "compare",
    ];
    if (!mac) file.push("-", "quit");
    const about: Entry = {
      about: appName,
      version: __APP_VERSION__,
      text: L("About %@", appName),
      website: downloadPage,
      websiteLabel: L("Download page"),
      comments: L("An editor of CSV files and Excel workbooks that keeps files as they were"),
    };
    const app = {
      title: appName,
      entries: [
        about,
        "-",
        { native: "Services", text: L("Services") },
        "-",
        { native: "Hide", text: L("Hide %@", appName) },
        { native: "HideOthers", text: L("Hide Others") },
        { native: "ShowAll", text: L("Show All") },
        "-",
        "quit",
      ] as Entry[],
    };
    const menus = [
      { title: L("File"), entries: file },
      {
        title: L("Edit"),
        entries: [
          "undo",
          "redo",
          "-",
          { native: "Cut", text: L("Cut") },
          { native: "Copy", text: L("Copy") },
          { native: "Paste", text: L("Paste") },
          { title: L("Copy As"), entries: ["copyAs:markdown", "copyAs:json", "copyAs:html", "copyAs:sql"] },
          "clear",
          "selectAll",
          "-",
          { title: L("Find"), entries: ["find", "findReplace", "findNext", "findPrevious"] },
        ] as Entry[],
      },
      {
        title: L("Table"),
        entries: [
          "editCell",
          "editInWindow",
          "fillDown",
          "-",
          "insertRowAbove",
          "insertRowBelow",
          "deleteRows",
          "moveRowsUp",
          "moveRowsDown",
          "-",
          "insertColumnLeft",
          "insertColumnRight",
          "deleteColumn",
          "renameColumn",
          "moveColumnLeft",
          "moveColumnRight",
          "-",
          "sortAscending",
          "sortDescending",
          "clearSort",
          "reorderRows",
          "-",
          "toggleFilter",
          "clearFilter",
          "toggleHeader",
        ] as Entry[],
      },
    ];
    menus.push({
      title: L("Data"),
      entries: [
        { title: L("Text"), entries: ["text:trim", "text:collapseSpaces", "-", "text:uppercase", "text:lowercase", "text:capitalize"] },
        "-",
        "fillFromAbove",
        "fillEmpty",
        "-",
        "splitColumn",
        "mergeColumns",
        "computedColumn",
        "-",
        "fixGarbled",
        "removeDuplicates",
      ] as Entry[],
    });
    menus.push({
      title: L("Recipes"),
      entries: [
        "recording",
        "-",
        ...(recipes.length ? recipes.map((recipe) => `recipe:${recipe.file}`) : ["noRecipes"]),
        "-",
        "manageRecipes",
      ] as Entry[],
    });
    menus.push({
      title: L("View"),
      entries: [
        "toggleProfile",
        "toggleRepair",
        "toggleCard",
        "previousRecord",
        "nextRecord",
        "previousSheet",
        "nextSheet",
        "-",
        "freezeColumns",
        "unfreezeColumns",
        "freezeRows",
        "unfreezeRows",
        "-",
        "hideColumn",
        "showAllColumns",
        "fitColumns",
        "-",
        "toggleInvisibles",
        { title: L("Text Wrapping"), entries: ["lines:1", "lines:2", "lines:3", "lines:5"] },
      ] as Entry[],
    });
    if (mac) {
      menus.unshift(app);
      menus.push({
        title: L("Window"),
        entries: [{ native: "Minimize", text: L("Minimize") }, { native: "Maximize", text: L("Zoom") }, "-", { native: "Fullscreen", text: L("Enter Full Screen") }],
      });
    } else {
      // Windows and Linux have no menu of the app: About is under Help, as there.
      menus.push({ title: L("Help"), entries: [about] });
    }
    return menus;
  }

  /** Builds the menu bar again, as when the recent files change. */
  async function installMenu() {
    commands = new Commands(commandList());
    if (!inTauri) return;
    const menu = new NativeMenu(commands);
    await menu.install(menuEntries());
    installed = menu;
  }
  let installed: NativeMenu | null = null;

  // The menu follows the document: what can be done and what is checked.
  $effect(() => {
    void doc.status;
    void selection;
    void finding;
    void filtering;
    void panel;
    void layoutChanged;
    void installed?.refresh();
  });

  const shortcutOf = (id: string) => {
    const shortcut = commands.get(id)?.shortcut;
    return shortcut ? shortcutLabel(shortcut) : undefined;
  };

  /** A command as an item of a menu at the pointer or under a toolbar button. */
  function menuItem(id: string): MenuItem | null {
    const command = commands.get(id);
    if (!command) return null;
    return {
      label: labelOf(command),
      shortcut: shortcutOf(id),
      disabled: !(command.enabled?.() ?? true),
      checked: command.checked?.(),
      run: () => command.run(),
    };
  }

  /** A menu of commands; "-" separates groups. */
  const menuOf = (ids: string[]) => ids.map((id) => (id === "-" ? null : menuItem(id)));

  /** The tooltip of the macOS app with this system's shortcut in place of the Mac's. */
  const tip = (text: string, id: string) => {
    const shortcut = shortcutOf(id);
    const words = L(text).replace(/\s*\([^)]*\)$/, "");
    return shortcut ? `${words} (${shortcut})` : words;
  };

  const toolbar = $derived.by((): (ToolbarItem | null)[] => {
    void doc.status;
    void selection;
    void filtering;
    void panel;
    const enabled = (id: string) => commands.get(id)?.enabled?.() ?? true;
    const view = doc.status?.view;
    return [
      { id: "insertRow", label: L("Insert Row"), tooltip: tip("Insert a row below (⌥⌘↓)", "insertRowBelow"), icon: "insertRow", enabled: doc.open && enabled("insertRowBelow"), run: () => void insertRow(true) },
      {
        id: "insertColumn",
        label: L("Insert Column"),
        tooltip: tip("Insert a column to the right (⌥⌘→)", "insertColumnRight"),
        icon: "insertColumn",
        enabled: doc.open && enabled("insertColumnRight"),
        run: () => void insertColumn(true),
      },
      { id: "deleteRows", label: L("Delete Rows"), tooltip: tip("Delete the selected rows (⌘⌫)", "deleteRows"), icon: "trash", enabled: enabled("deleteRows"), run: () => void deleteRows() },
      null,
      {
        id: "sort",
        label: L("Sort"),
        tooltip: L("Sort by the active column"),
        icon: "sort",
        enabled: doc.open,
        active: (view?.sort.length ?? 0) > 0,
        menu: () => menuOf(["sortAscending", "sortDescending", "clearSort", "-", "reorderRows"]),
      },
      {
        id: "filter",
        label: L("Filter"),
        tooltip: tip("Show or hide the filter (⇧⌘L)", "toggleFilter"),
        icon: "filter",
        enabled: doc.open && idleOrView(),
        active: (view?.filters.length ?? 0) > 0,
        run: toggleFilter,
      },
      {
        id: "data",
        label: L("Data"),
        tooltip: L("Clean up text, fill empty cells, split and merge columns"),
        icon: "data",
        enabled: doc.open,
        menu: () =>
          menuOf([
            "text:trim",
            "text:collapseSpaces",
            "text:uppercase",
            "text:lowercase",
            "text:capitalize",
            "-",
            "fillFromAbove",
            "fillEmpty",
            "-",
            "splitColumn",
            "mergeColumns",
            "computedColumn",
            "-",
            "fixGarbled",
            "removeDuplicates",
          ]),
      },
      {
        id: "export",
        label: L("Export"),
        tooltip: L("Export for Excel, JSON, Markdown, HTML, SQL"),
        icon: "export",
        enabled: doc.open,
        menu: () => menuOf(["exportExcel", "exportCopy", "-", "exportAs:json", "exportAs:markdown", "exportAs:html", "exportAs:sql", "exportAs:xlsx"]),
      },
      {
        id: "profile",
        label: L("Profile"),
        tooltip: tip("Column profile: type, statistics, problems (⌥⌘P)", "toggleProfile"),
        icon: "profile",
        enabled: (doc.status?.columns ?? 0) > 0,
        active: panel === "profile",
        run: toggleProfile,
      },
      {
        id: "card",
        label: L("Card"),
        tooltip: tip("The active row as a form (⌥⌘I)", "toggleCard"),
        icon: "card",
        enabled: (doc.status?.columns ?? 0) > 0,
        active: panel === "card",
        run: toggleCard,
      },
      {
        id: "repair",
        label: L("Repair"),
        tooltip: tip("Find and fix broken rows (⌥⌘R)", "toggleRepair"),
        icon: "repair",
        enabled: doc.open,
        active: panel === "repair",
        run: toggleRepair,
      },
    ];
  });
  const idleOrView = () => !!doc.status && doc.status.indexing === null && doc.status.job?.kind !== "edit";

  /** Keys that reach the grid itself and are neither moves of the cursor nor commands. */
  function gridKey(event: KeyboardEvent): boolean {
    const cursor = grid?.selection?.cursor;
    const plain = !event.metaKey && !event.ctrlKey && !event.altKey;
    if (!cursor || !plain) return false;
    // F2 edits too, as in spreadsheets on Windows and Linux.
    if ((event.key === "Enter" && !event.shiftKey) || (event.key === "F2" && !event.shiftKey)) void edit(cursor);
    else if (event.key === "Delete" || event.key === "Backspace") void clear();
    else return false;
    return true;
  }

  function keydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (grid?.owns(event.target) && gridKey(event)) {
      event.preventDefault();
      return;
    }
    commands.key(event);
  }

  onMount(() => {
    grid = new Grid(host);
    editor = new CellEditor(grid, host, (cell, value, move) => void write(cell, value, move));
    grid.onActivate = (cell) => void edit(cell);
    // Typing replaces the value, as in the macOS app.
    grid.onType = (text) => {
      const cursor = grid?.selection?.cursor;
      if (cursor) void edit(cursor, text);
    };
    grid.onCopy = (cut) => void copy(cut);
    grid.onPaste = (text) => void paste(text);
    grid.onMenu = openMenu;
    grid.onHeader = headerClick;
    grid.onFilterIcon = (column) => void filterColumn(column);
    grid.onMoveRows = (before) => void moveRows(before);
    grid.onMoveColumn = (from, to) => void moveColumn(from, to);
    grid.canMoveRows = canMoveRows;
    grid.missingText = L("This row has no such field");
    grid.onSelect = (selected) => {
      const column = selection?.cursor.column;
      selection = selected;
      void measure(selected);
      if (selected && selected.cursor.column !== column) refreshProfile();
      void repairCursorMoved();
      void refreshCard();
      void inspectCell();
    };
    const parameters = new URLSearchParams(location.search);
    void loadRecipes();
    if (inTauri) {
      const window = getCurrentWindow();
      void window.onCloseRequested(async (event) => {
        event.preventDefault();
        if (await confirmClose()) await window.destroy();
      });
      void installMenu();
      // macOS has one menu bar: the window in front gives it its commands.
      void window.onFocusChanged(({ payload: focused }) => {
        if (focused) void installMenu();
      });
      void window.listen<string[]>("open-files", (event) => void openPaths(event.payload));
      void window.listen<number>("reveal-row", (event) => void revealRow(event.payload));
      void getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop") void openPaths(event.payload.paths);
      });
    } else {
      addEventListener("beforeunload", (event) => {
        if (doc.status?.dirty) event.preventDefault();
      });
      new BroadcastChannel("comma-reveal").onmessage = (event: MessageEvent<{ window: string; row: number }>) => {
        if (event.data.window === windowName()) void revealRow(event.data.row);
      };
      // A browser has no menu bar: tests and the console run the menus' commands by name.
      (window as unknown as { commaCommand: (id: string) => void }).commaCommand = (id) => commands.menu(id);
    }
    if (parameters.has("adopt")) {
      fresh = true;
      void call<Status>("adopt", { token: Number(parameters.get("adopt")) }).then((status) => doc.show(status), (error) => (failure = describe(error)));
    } else if (parameters.has("draft")) void restoreDraft(parameters.get("draft")!);
    else if (parameters.has("open")) void show(parameters.get("open")!);
    else if (parameters.has("new")) void newDocument();
    else if (inTauri) {
      void import("./lib/bench-launch").then(({ launch, benchDone }) =>
        launch().then(async ({ paths, bench, drafts }) => {
          if (bench) void (paths[0] ? benchAndQuit(paths[0]) : benchDone(false, "--bench: no file"));
          else {
            await offerDrafts(drafts);
            await openPaths(paths);
          }
        }),
      );
    }
  });
</script>

<svelte:window onkeydown={keydown} />

<div class="app">
  <header class="toolbar">
    <button onclick={choose} title={openShortcut}>{L("Open…")}</button>
    <Toolbar items={toolbar} openMenu={(x, y, items) => (menu = { x, y, items })} />
  </header>
  {#if finding && doc.status}
    <FindBar
      bind:this={findBar}
      document={doc}
      replacing={finding.replacing}
      selection={finding.selection}
      titles={Array.from({ length: doc.status.columns }, (_, column) => grid?.title(column) ?? "")}
      cursor={() => grid?.selection?.cursor ?? null}
      select={(cell) => grid?.select(Selection.at(cell))}
      close={() => {
        finding = null;
        grid?.focus();
      }}
    />
  {/if}
  {#if filtering && doc.status}
    <FilterBar
      bind:this={filterBar}
      filters={doc.status.view.filters}
      titles={Array.from({ length: doc.status.columns }, (_, column) => grid?.title(column) ?? "")}
      problem={filterProblem}
      apply={setFilters}
      close={toggleFilter}
    />
  {/if}
  <div class="body">
  <main class="table" bind:this={host}>
    {#if !doc.open}
      <div class="empty">
        {#if failure}
          <p class="error">{failure.message}</p>
          {#if failure.detail}<p class="detail">{failure.detail}</p>{/if}
        {/if}
        <p>{L("Open a CSV file: %@", openShortcut)}</p>
      </div>
    {/if}
  </main>
  {#if panel === "profile"}
    <ProfilePanel
      name={profiledName}
      {profile}
      loading={profiling}
      {showIssue}
      trim={trimProfiled}
      {filterValue}
    />
  {:else if panel === "card"}
    <RecordCard
      row={card?.row ?? null}
      title={card?.title ?? ""}
      position={card?.position ?? ""}
      fields={card?.fields ?? []}
      activeColumn={selection?.cursor.column ?? 0}
      canGoBack={(card?.row ?? 0) > 0}
      canGoForward={(card?.row ?? 0) < (doc.status?.rows ?? 0) - 1}
      edit={(row, column, value) => void doc.attempt(() => doc.run("set_cell", { row, column, value }))}
      selectColumn={(row, column) => {
        const cursor = grid?.selection?.cursor;
        if (cursor?.row !== row || cursor?.column !== column) grid?.select(Selection.at({ row, column }));
      }}
      previous={() => moveRecord(-1)}
      next={() => moveRecord(1)}
    />
  {:else if panel === "repair"}
    <RepairPanel
      report={repair}
      loading={repairLoading}
      selectedRow={repairRow}
      rawText={repairRaw}
      options={repairOptions}
      {garbledExample}
      rowLabel={repairRowLabel}
      columnTitle={(column) => grid?.title(column) ?? ""}
      select={(problem) => void selectProblem(problem, true)}
      apply={(row, fix) => void applyFix(row, fix)}
      fixAll={(kind) => void fixAll(kind)}
      fixGarbled={() => void change(L("Fixing garbled text…"), { fixGarbled: { range: null } })}
    />
  {/if}
  </div>
  {#if doc.status?.book}
    <SheetTabs
      sheets={doc.status.book.sheets}
      selected={doc.status.book.index}
      loading={doc.status.book.loading?.index ?? null}
      csvMode={doc.status.book.csvMode}
      modeEnabled={idle() && doc.status.dataRows + (doc.status.hasHeader ? 1 : 0) > 0}
      select={(index) => void chooseSheet(index)}
      setMode={() => void toggleHeader()}
    />
  {/if}
  <footer class="status">
    <span class="info">{doc.problem || status}</span>
    {#if problemsLabel}
      <button class="problems" onclick={toggleRepair} title={tip("Find and fix broken rows (⌥⌘R)", "toggleRepair")}>⚠︎ {problemsLabel}</button>
    {/if}
    {#if busy}
      <span class="busy">{busy}</span>
      {#if doc.status?.job || doc.status?.book?.loading}
        <button class="small" onclick={() => doc.run("cancel_job")}>{L("Cancel")}</button>
      {/if}
    {/if}
  </footer>
  {#if menu}
    <ContextMenu
      x={menu.x}
      y={menu.y}
      items={menu.items}
      close={() => {
        menu = null;
        grid?.focus();
      }}
    />
  {/if}
  {#if managing}
    <RecipesSheet
      {recipes}
      canApply={idle()}
      rename={(file, name) => call("recipe_rename", { file, name }).then(loadRecipes)}
      removeStep={(file, index) => call("recipe_remove_step", { file, index }).then(loadRecipes)}
      remove={(file) => call("recipe_delete", { file }).then(loadRecipes)}
      exportPython={(recipe) => void exportPython(recipe)}
      apply={(recipe) => void applyRecipe(recipe)}
      close={() => {
        managing = false;
        grid?.focus();
      }}
    />
  {/if}
  {#if computing}
    <FormulaSheet
      columns={computing.columns}
      activeColumnName={computing.name}
      apply={(choice) => {
        const column = computing!.column;
        computing = null;
        grid?.focus();
        void applyFormula(choice, column);
      }}
      cancel={() => {
        computing = null;
        grid?.focus();
      }}
    />
  {/if}
  {#if checking && doc.status?.path}
    <OpenOptions
      path={doc.status.path}
      encoding={reading.encoding}
      delimiter={reading.delimiter}
      uncertain={checking.uncertain}
      choose={(choice) => {
        checking = null;
        grid?.focus();
        if (choice) void reopen(choice.encoding, choice.delimiter);
      }}
    />
  {/if}
  {#if alert}
    <Alert
      message={alert.message}
      detail={alert.detail}
      buttons={alert.buttons}
      field={alert.field}
      fieldLabel={alert.fieldLabel}
      placeholder={alert.placeholder}
      checkbox={alert.checkbox}
      choose={(value, text, checked) => {
        const answer = alert!.answer;
        alert = null;
        grid?.focus();
        answer(value, text, checked);
      }}
    />
  {/if}
  {#if saving}
    <SaveSheet
      format={saving.format}
      title={saving.title}
      excel={saving.excel}
      choose={(format) => {
        const answer = saving!.answer;
        saving = null;
        grid?.focus();
        answer(format);
      }}
    />
  {/if}
  {#if sheet}
    <CellSheet
      heading={sheet.heading}
      value={sheet.value}
      save={(value) => {
        const cell = sheet!.cell;
        const changed = value !== sheet!.value;
        sheet = null;
        grid?.focus();
        if (changed) void write(cell, value, null);
      }}
      cancel={() => {
        sheet = null;
        grid?.focus();
      }}
    />
  {/if}
  {#if benchText}
    <aside class="bench">
      <pre>{benchText}</pre>
      <div class="actions">
        <button onclick={() => navigator.clipboard.writeText(benchText)}>{L("Copy")}</button>
        <button onclick={() => (benchText = "")}>{L("Close")}</button>
      </div>
    </aside>
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: var(--bar);
    border-bottom: 1px solid var(--border);
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .table {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--grid-background);
  }

  .empty {
    position: absolute;
    inset: 0;
    z-index: 1;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--muted);
    background: var(--background);
  }

  .error {
    color: var(--error);
    max-width: 60ch;
    margin-bottom: 0;
  }

  .detail {
    max-width: 60ch;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 3px 10px;
    min-height: 22px;
    color: var(--muted);
    background: var(--bar);
    border-top: 1px solid var(--border);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .info {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .problems {
    padding: 0 4px;
    color: var(--text);
    background: none;
    border: 0;
  }

  .bench {
    position: absolute;
    right: 16px;
    bottom: 40px;
    z-index: 2;
    max-width: min(720px, calc(100% - 32px));
    padding: 12px;
    background: var(--bar);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.18);
  }

  .bench pre {
    margin: 0 0 8px;
    white-space: pre-wrap;
    font: 12px ui-monospace, "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>

<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open, save as chooseSavePath } from "@tauri-apps/plugin-dialog";
  import { Grid, type TableSource } from "./grid/Grid";
  import { formatBench, runBench } from "./grid/bench";
  import { call, inTauri, type RangeStats, type SaveFormat, type Status } from "./lib/api";
  import { Selection, type Cell } from "./grid/selection";
  import { CellEditor, type Move } from "./grid/editor";
  import CellSheet from "./CellSheet.svelte";
  import Alert, { type Button } from "./Alert.svelte";
  import SaveSheet from "./SaveSheet.svelte";
  import { Document } from "./lib/document.svelte";
  import { describe } from "./lib/errors";
  import { L, count, locale } from "./lib/i18n";
  import { readClipboard, writeClipboard } from "./lib/clipboard";
  import { shortcutLabel, type Shortcut } from "./lib/keys";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import type { CellRange } from "./lib/api";

  const mac = navigator.userAgent.includes("Mac");
  const openShortcut = mac ? "⌘O" : "Ctrl+O";

  let host: HTMLElement;
  let grid: Grid | null = null;
  let editor: CellEditor | null = null;
  /** A question or a problem shown over the table, and who waits for the answer. */
  let alert = $state<{
    message: string;
    detail?: string;
    buttons: Button[];
    field?: string;
    answer: (value: string, text: string) => void;
  } | null>(null);
  /** The format of a file being saved, asked before the file is chosen. */
  let saving = $state<{ format: SaveFormat; answer: (format: SaveFormat | null) => void } | null>(null);
  /** A menu open at the pointer. */
  let menu = $state<{ x: number; y: number; items: (MenuItem | null)[] } | null>(null);
  /** A value with line breaks being edited in a window of its own. */
  let sheet = $state<{ cell: Cell; heading: string; value: string } | null>(null);
  /** The status the view shows rows for; the rows reload when it differs in what they show. */
  const doc = new Document((status, previous) => {
    if (!grid) return;
    if (!previous || previous.path !== status.path) grid.setSource(table);
    else if (
      previous.change !== status.change ||
      previous.hasHeader !== status.hasHeader ||
      JSON.stringify(previous.view) !== JSON.stringify(status.view)
    )
      grid.reload();
    else if (previous.rows !== status.rows || previous.columns !== status.columns) grid.update();
    void showTitle(status);
  });
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
    fetch: (start, count) => call<string[][]>("rows", { start, count }),
  };

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
    if (selection && !selection.single) {
      parts.push(L("Selected: %@ × %@", count(selection.rowCount), count(selection.columnCount)));
      if (stats && stats.numbers > 0) {
        const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 6 });
        parts.push(L("Sum: %@", number.format(stats.sum)));
        parts.push(L("Average: %@", number.format(stats.sum / stats.numbers)));
      }
    }
    if (status.view.filters.length > 0) parts.push(L("Showing %@ of %@", count(status.rows), count(status.dataRows)));
    else parts.push(L("Rows: %@", count(status.rows)));
    parts.push(L("Columns: %@", count(status.columns)));
    parts.push(status.hasBom ? L("%@ with BOM", status.encoding) : status.encoding);
    const delimiter = status.delimiter === "\t" ? L("Tab") : status.delimiter === " " ? L("Space") : status.delimiter;
    parts.push(L("Delimiter: %@", delimiter));
    parts.push({ lf: "LF", crLf: "CRLF", cr: "CR" }[status.lineEnding]);
    return parts.join("   ·   ");
  });

  const busy = $derived.by(() => {
    const status = doc.status;
    if (!status) return "";
    if (status.indexing !== null) return L("Reading the file: %@", `${Math.floor(status.indexing * 100)} %`);
    if (status.job?.kind === "view") return L("Sorting and filtering…");
    if (status.job?.kind === "edit") return `${L("Replacing…")} ${Math.floor((status.job.fraction ?? 0) * 100)} %`;
    return "";
  });

  /** Opens a file in this window; the time to the first screen goes to the bench. */
  async function show(path: string): Promise<boolean> {
    failure = null;
    firstScreen = null;
    const started = performance.now();
    if (grid) {
      grid.onFrame = (complete) => {
        if (complete && firstScreen === null && doc.open) firstScreen = performance.now() - started;
      };
    }
    try {
      await doc.run("open", { path });
      openSeconds = (performance.now() - started) / 1000;
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
            { name: "CSV", extensions: ["csv", "tsv", "tab", "txt"] },
            { name: "*", extensions: ["*"] },
          ],
        })
      : await askText(L("Open…"), doc.status?.path ?? "", L("Open"));
    if (path) await show(path);
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
  async function benchAndQuit(path: string) {
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
      return true;
    } catch (error) {
      await report(error);
      return false;
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

  /** Whether the document can change now; says why not in the status bar. */
  function changeable(): boolean {
    const status = doc.status;
    if (!status) return false;
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
    const [row] = await call<string[][]>("rows", { start: cell.row, count: 1 });
    return row?.[cell.column] ?? "";
  }

  /** Edits a cell: in place, or in a window when its value has line breaks or `window` asks. */
  async function edit(cell: Cell, typed?: string, inWindow = false) {
    if (!grid || !editor || !changeable()) return;
    const value = await cellValue(cell);
    if (inWindow || value.includes("\n") || value.includes("\r")) {
      sheet = { cell, heading: L("Row %ld, column %@", cell.row + 1, grid.title(cell.column)), value };
      return;
    }
    editor.start(cell, value, typed);
  }

  async function write(cell: Cell, value: string | null, move: Move) {
    if (value !== null) await doc.attempt(() => doc.run("set_cell", { row: cell.row, column: cell.column, value }));
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
    if (await doc.attempt(() => doc.run("insert_column", { at }))) grid.select(Selection.at({ row: cursor.row, column: at }));
  }

  async function deleteColumn() {
    const cursor = grid?.selection?.cursor;
    if (!grid || !cursor || (doc.status?.columns ?? 0) < 2 || !changeable()) return;
    await doc.attempt(() => doc.run("delete_column", { at: cursor.column }));
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

  async function toggleHeader() {
    const status = doc.status;
    if (!status || !changeable()) return;
    await keepingRow(() => doc.run("set_has_header", { hasHeader: !status.hasHeader }));
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

  const keys = (shortcut: Shortcut) => shortcutLabel(shortcut);

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
          { label: L("Rename Column…"), disabled: busy || !status.hasHeader, run: () => void renameColumn(column) },
          null,
          { label: L("Insert Column Left"), shortcut: keys({ key: "ArrowLeft", command: true, alt: true }), disabled: busy, run: () => void insertColumn(false) },
          { label: L("Insert Column Right"), shortcut: keys({ key: "ArrowRight", command: true, alt: true }), disabled: busy, run: () => void insertColumn(true) },
          { label: L("Delete Column"), shortcut: keys({ key: "Backspace", command: true, alt: true }), disabled: busy || status.columns < 2, run: () => void deleteColumn() },
          null,
          { label: L("First Row Is Header"), checked: status.hasHeader, disabled: busy, run: () => void toggleHeader() },
        ],
      };
      return;
    }
    const cell = target.cell;
    const view = sortedOrFiltered();
    menu = {
      x,
      y,
      items: [
        { label: L("Edit Cell"), disabled: busy, run: () => void edit(cell) },
        { label: L("Edit Cell in Window…"), shortcut: keys({ key: "e", command: true, alt: true }), disabled: busy, run: () => void edit(cell, undefined, true) },
        null,
        { label: L("Cut"), shortcut: keys({ key: "x", command: true }), disabled: busy, run: () => void copy(true) },
        { label: L("Copy"), shortcut: keys({ key: "c", command: true }), run: () => void copy(false) },
        { label: L("Paste"), shortcut: keys({ key: "v", command: true }), disabled: busy, run: () => void readClipboard().then(paste) },
        { label: L("Fill Down"), shortcut: keys({ key: "d", command: true }), disabled: busy, run: () => void fillDown() },
        null,
        { label: L("Insert Row Above"), shortcut: keys({ key: "ArrowUp", command: true, alt: true }), disabled: busy || view, run: () => void insertRow(false) },
        { label: L("Insert Row Below"), shortcut: keys({ key: "ArrowDown", command: true, alt: true }), disabled: busy || view, run: () => void insertRow(true) },
        { label: L("Delete Rows"), shortcut: keys({ key: "Backspace", command: true }), disabled: busy, run: () => void deleteRows() },
      ],
    };
  }

  /** Keys that reach the grid itself and are not moves of the cursor. */
  function gridKey(event: KeyboardEvent): boolean {
    const command = mac ? event.metaKey : event.ctrlKey;
    const cursor = grid?.selection?.cursor;
    const key = event.key;
    if (command && !event.altKey && key.toLowerCase() === "z") {
      void undo(event.shiftKey);
    } else if (!mac && event.ctrlKey && !event.shiftKey && !event.altKey && key.toLowerCase() === "y") {
      void undo(true);
    } else if (command && event.altKey && event.code === "KeyE") {
      if (cursor) void edit(cursor, undefined, true);
    } else if (command && event.altKey && (key === "ArrowUp" || key === "ArrowDown")) {
      void insertRow(key === "ArrowDown");
    } else if (command && event.altKey && (key === "ArrowLeft" || key === "ArrowRight")) {
      void insertColumn(key === "ArrowRight");
    } else if (command && event.altKey && key === "Backspace") {
      void deleteColumn();
    } else if (command && !event.altKey && !event.shiftKey && key === "Backspace") {
      void deleteRows();
    } else if (command && !event.altKey && !event.shiftKey && event.code === "KeyD") {
      void fillDown();
    } else if (!cursor) {
      return false;
    } else if (key === "Enter" && !command && !event.altKey && !event.shiftKey) {
      void edit(cursor);
    } else if ((key === "Delete" || key === "Backspace") && !command && !event.altKey) {
      void clear();
    } else {
      return false;
    }
    return true;
  }

  function keydown(event: KeyboardEvent) {
    if (grid?.owns(event.target) && !event.isComposing && gridKey(event)) {
      event.preventDefault();
      return;
    }
    const command = mac ? event.metaKey : event.ctrlKey;
    if (command && !event.altKey && event.code === "KeyS" && doc.open) {
      event.preventDefault();
      void (event.shiftKey ? saveAs() : save());
      return;
    }
    if (command && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "o") {
      event.preventDefault();
      void choose();
    } else if (command && event.shiftKey && event.altKey && event.code === "KeyB") {
      event.preventDefault();
      void bench().then((text) => (benchText = text));
    }
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
    grid.onSelect = (selected) => {
      selection = selected;
      void measure(selected);
    };
    if (inTauri) {
      void getCurrentWindow().onCloseRequested(async (event) => {
        event.preventDefault();
        if (await confirmClose()) await getCurrentWindow().destroy();
      });
    } else {
      addEventListener("beforeunload", (event) => {
        if (doc.status?.dirty) event.preventDefault();
      });
    }
    if (inTauri) {
      void import("./lib/bench-launch").then(({ launch }) =>
        launch().then(({ path, bench }) => {
          if (bench) void (path ? benchAndQuit(path) : import("./lib/bench-launch").then((m) => m.benchDone(false, "--bench: no file")));
          else if (path) void show(path);
        }),
      );
    } else {
      const path = new URLSearchParams(location.search).get("open");
      if (path) void show(path);
    }
  });
</script>

<svelte:window onkeydown={keydown} />

<div class="app">
  <header class="toolbar">
    <button onclick={choose} title={openShortcut}>{L("Open…")}</button>
  </header>
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
  <footer class="status">
    <span class="info">{doc.problem || status}</span>
    {#if busy}
      <span class="busy">{busy}</span>
      {#if doc.status?.job}
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
  {#if alert}
    <Alert
      message={alert.message}
      detail={alert.detail}
      buttons={alert.buttons}
      field={alert.field}
      choose={(value, text) => {
        const answer = alert!.answer;
        alert = null;
        grid?.focus();
        answer(value, text);
      }}
    />
  {/if}
  {#if saving}
    <SaveSheet
      format={saving.format}
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
    display: grid;
    grid-template-rows: auto 1fr auto;
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

  .table {
    position: relative;
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

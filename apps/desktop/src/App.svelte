<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Grid, type TableSource } from "./grid/Grid";
  import { formatBench, runBench } from "./grid/bench";
  import { call, inTauri, type Status } from "./lib/api";
  import { Document } from "./lib/document.svelte";
  import { describe } from "./lib/errors";
  import { L, count } from "./lib/i18n";

  const mac = navigator.userAgent.includes("Mac");
  const openShortcut = mac ? "⌘O" : "Ctrl+O";

  let host: HTMLElement;
  let grid: Grid | null = null;
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
      : prompt(L("Open…"));
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

  function keydown(event: KeyboardEvent) {
    const command = mac ? event.metaKey : event.ctrlKey;
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

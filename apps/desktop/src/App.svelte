<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Grid } from "./grid/Grid";
  import { benchDone, benchStep, demoFile, inTauri, launch, openFile, type OpenedFile } from "./grid/source";
  import { formatBench, runBench } from "./grid/bench";

  const mac = navigator.userAgent.includes("Mac");
  const openShortcut = mac ? "⌘O" : "Ctrl+O";
  const number = new Intl.NumberFormat("ru-RU");
  const seconds = new Intl.NumberFormat("ru-RU", { minimumFractionDigits: 3, maximumFractionDigits: 3 });

  let host: HTMLElement;
  let grid: Grid | null = null;
  // The file changes outside Svelte while it is indexed; `tick` redraws the status bar.
  let file = $state.raw<OpenedFile | null>(null);
  let tick = $state(0);
  let error = $state("");
  let firstScreen = $state<number | null>(null);
  let benching = $state(false);
  let benchText = $state("");

  const status = $derived.by(() => {
    void tick;
    if (!file) return "";
    const parts = [
      file.name,
      `${number.format(file.rows)} строк`,
      `${number.format(file.columns)} колонок`,
      file.encoding,
      `«${file.delimiter === "\t" ? "Tab" : file.delimiter}»`,
    ];
    if (file.indexed < 1) parts.push(`чтение ${Math.floor(file.indexed * 100)} %`);
    if (inTauri) parts.push(`открытие ${seconds.format(file.openSeconds)} с`);
    if (firstScreen !== null) parts.push(`первый экран ${seconds.format(firstScreen / 1000)} с`);
    if (grid && grid.stats.drawing.count > 0) parts.push(`кадр p95 ${grid.stats.drawing.percentile(0.95).toFixed(1)} мс`);
    if (grid && grid.stats.requests.count > 0) parts.push(`запрос p95 ${grid.stats.requests.percentile(0.95).toFixed(1)} мс`);
    return parts.join(" · ");
  });

  async function show(path: string) {
    error = "";
    firstScreen = null;
    const started = performance.now();
    try {
      const opened = await openFile(path, () => {
        grid?.update();
        tick += 1;
      });
      present(opened, started);
    } catch (problem) {
      error = String(problem);
    }
  }

  function present(opened: OpenedFile, started: number) {
    file = opened;
    if (!grid) return;
    grid.onFrame = (complete) => {
      if (complete && firstScreen === null) firstScreen = performance.now() - started;
    };
    grid.setSource(opened);
  }

  async function choose() {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Таблицы", extensions: ["csv", "tsv", "tab", "txt"] },
        { name: "Все файлы", extensions: ["*"] },
      ],
    });
    if (path) await show(path);
  }

  async function bench() {
    if (!grid || !file) return;
    benching = true;
    benchText = "";
    try {
      const result = await runBench(grid);
      const opening = [
        `открытие ${seconds.format(file.openSeconds)} с`,
        firstScreen === null ? "" : `первый экран ${seconds.format(firstScreen / 1000)} с`,
      ].filter(Boolean);
      const where = [
        navigator.userAgent,
        `${file.name}: ${number.format(file.rows)} строк, окно ${innerWidth}×${innerHeight}, масштаб ${devicePixelRatio}`,
        opening.join(", "),
      ].join("\n");
      benchText = formatBench(result, where);
      console.info(benchText);
    } finally {
      benching = false;
    }
  }

  /** `--bench FILE`: opens the file, waits for it to be read whole, times scrolling and quits. */
  async function benchAndQuit(path: string) {
    void benchStep(`opening ${path}`);
    await show(path);
    if (!file) {
      await benchDone(false, `не удалось открыть ${path}: ${error}`);
      return;
    }
    void benchStep(`opened, ${file.rows} rows so far; reading the rest`);
    while (file.indexed < 1 || firstScreen === null) await new Promise((resolve) => setTimeout(resolve, 50));
    void benchStep(`read ${file.rows} rows; scrolling`);
    try {
      await bench();
      await benchDone(true, benchText);
    } catch (problem) {
      await benchDone(false, `замер не удался: ${problem}`);
    }
  }

  function keydown(event: KeyboardEvent) {
    if ((mac ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === "o" && inTauri) {
      event.preventDefault();
      void choose();
    }
  }

  onMount(() => {
    grid = new Grid(host);
    const timer = setInterval(() => (tick += 1), 500);
    if (inTauri) {
      void launch().then(({ path, bench }) => {
        if (bench) void (path ? benchAndQuit(path) : benchDone(false, "--bench: укажите файл"));
        else if (path) void show(path);
      });
    } else {
      present(demoFile(), performance.now());
    }
    return () => clearInterval(timer);
  });
</script>

<svelte:window onkeydown={keydown} />

<div class="app">
  <header class="toolbar">
    {#if inTauri}
      <button onclick={choose} title={openShortcut}>Открыть…</button>
    {/if}
    <button onclick={bench} disabled={!file || benching}>{benching ? "Замер…" : "Замер прокрутки"}</button>
    {#if !inTauri}
      <span class="note">Демо в браузере: строки придумываются на лету</span>
    {/if}
  </header>
  <main class="table" bind:this={host}>
    {#if !file}
      <div class="empty">
        {#if error}
          <p class="error">{error}</p>
        {/if}
        <p>Откройте CSV-файл: {openShortcut}</p>
      </div>
    {/if}
  </main>
  <footer class="status">{error && file ? error : status}</footer>
  {#if benchText}
    <aside class="bench">
      <pre>{benchText}</pre>
      <div class="actions">
        <button onclick={() => navigator.clipboard.writeText(benchText)}>Скопировать</button>
        <button onclick={() => (benchText = "")}>Закрыть</button>
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

  .note {
    color: var(--muted);
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
    color: #d70015;
    max-width: 60ch;
  }

  .status {
    padding: 4px 10px;
    color: var(--muted);
    background: var(--bar);
    border-top: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
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

  button {
    font: inherit;
    padding: 4px 12px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  button:disabled {
    color: var(--muted);
  }
</style>

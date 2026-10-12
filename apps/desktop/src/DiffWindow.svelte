<!-- A comparison of a window's table with another file (DiffWindowController in the macOS app):
     rows matched by a key or by position, with what was only on one side or changed. -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { emitTo } from "@tauri-apps/api/event";
  import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
  import { call, inTauri } from "./lib/api";
  import { describe } from "./lib/errors";
  import { L, count } from "./lib/i18n";
  import { newWindow } from "./lib/windows";

  type DiffStatus = "onlyLeft" | "onlyRight" | "changed" | "same";
  interface Column {
    name: string | null;
    left: number | null;
    right: number | null;
  }
  interface Summary {
    onlyLeft: number;
    onlyRight: number;
    changed: number;
    same: number;
    repeatedLeft: number;
    repeatedRight: number;
  }
  interface State {
    reading: boolean;
    ready: boolean;
    comparing: number | null;
    cancelled: boolean;
    failure: unknown;
    columns: Column[];
    suggested: number[];
    summary: Summary | null;
    leftOffset: number;
    rightOffset: number;
  }
  interface Row {
    status: DiffStatus;
    leftRow: number | null;
    rightRow: number | null;
    cells: { left: string | null; right: string | null; changed: boolean }[];
  }

  interface Props {
    id: number;
    /** The window of the table compared, to show its rows in. */
    source: string;
    leftName: string;
    otherName: string;
  }

  let { id, source, leftName, otherName }: Props = $props();

  const ROW = 22;
  const PAGE = 200;
  const filters: DiffStatus[][] = [["onlyLeft", "onlyRight", "changed"], ["onlyLeft"], ["onlyRight"], ["changed"], ["same"]];

  let info = $state.raw<State | null>(null);
  let key = $state(-1);
  let ignoreCase = $state(false);
  let trim = $state(false);
  let filter = $state(0);
  let total = $state(0);
  let pages = $state.raw(new Map<number, Row[]>());
  let scroller = $state<HTMLElement>();
  let viewHeight = $state(400);
  let scrollTop = $state(0);
  let timer = 0;
  let started = false;
  let shownFor: Summary | null = null;

  /** The scrolled-to rows; the list's height is capped, past it the position is a share. */
  const MAX_HEIGHT = 8_000_000;
  const height = $derived(Math.min(total * ROW, MAX_HEIGHT));
  const first = $derived.by(() => {
    if (total * ROW <= MAX_HEIGHT) return Math.floor(scrollTop / ROW);
    const share = scrollTop / Math.max(1, height - viewHeight);
    return Math.floor(share * Math.max(0, total - Math.floor(viewHeight / ROW)));
  });
  const visible = $derived(Array.from({ length: Math.max(0, Math.min(total - first, Math.ceil(viewHeight / ROW) + 1)) }, (_, index) => first + index));

  const columns = $derived(info?.columns ?? []);
  const shared = $derived(columns.filter((column) => column.left !== null && column.right !== null));
  const letter = (index: number) => {
    let name = "";
    for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
    return name;
  };
  const columnName = (column: Column, index: number) => column.name || letter(column.left ?? column.right ?? index);
  const columnTitle = (column: Column, index: number) => {
    const name = columnName(column, index);
    if (column.left === null) return `${name} ${L("(only in %@)", otherName)}`;
    if (column.right === null) return `${name} ${L("(only in %@)", leftName)}`;
    return name;
  };
  const widths = $derived(`22px 110px ${columns.map(() => "150px").join(" ")}`);

  const filterLabels = $derived.by(() => {
    const summary = info?.summary;
    const labels = [L("All Differences"), L("Only in %@", leftName), L("Only in %@", otherName), L("Changed"), L("Same")];
    if (!summary) return labels;
    const counts = [summary.onlyLeft + summary.onlyRight + summary.changed, summary.onlyLeft, summary.onlyRight, summary.changed, summary.same];
    return labels.map((label, index) => `${label} (${count(counts[index])})`);
  });

  const busy = $derived(!!info && (info.reading || info.comparing !== null));

  const status = $derived.by(() => {
    if (!info) return "";
    if (info.reading) return L("Reading %@…", otherName);
    if (info.comparing !== null) return `${L("Comparing…")} ${Math.floor(info.comparing * 100)} %`;
    if (info.failure) return describe(info.failure).message;
    if (info.cancelled) return L("The comparison was cancelled.");
    const summary = info.summary;
    if (!summary) return "";
    const sentences: string[] = [];
    const differences = summary.onlyLeft + summary.onlyRight + summary.changed;
    sentences.push(differences === 0 ? L("The tables match.") : L("Differences: %@.", count(differences)));
    if (summary.repeatedLeft > 0 || summary.repeatedRight > 0) {
      sentences.push(
        L("Rows with a repeated key: %@ in %@, %@ in %@; they are paired in order.", count(summary.repeatedLeft), leftName, count(summary.repeatedRight), otherName),
      );
    }
    const oneSided = columns.filter((column) => column.left === null || column.right === null).length;
    if (oneSided > 0) sentences.push(L("Columns only in one file are shown but not compared: %@.", count(oneSided)));
    sentences.push(L("Double-click a row to show it in the table."));
    return sentences.join(" ");
  });

  /** Follows reading and comparing; a new result shows from the top. */
  async function poll() {
    timer = 0;
    try {
      info = await call<State>("diff_state", { id });
    } catch {
      return;
    }
    if (info.ready && !started) {
      started = true;
      key = info.suggested[0] ?? -1;
      await run();
    }
    if (info.summary && info.comparing === null && info.summary !== shownFor) {
      shownFor = info.summary;
      await reload();
    }
    if (info.reading || info.comparing !== null) timer = window.setTimeout(poll, 100);
  }

  async function run() {
    await call("diff_run", { id, options: { key: key < 0 ? [] : [key], ignoreCase, trim } });
    shownFor = null;
    if (!timer) timer = window.setTimeout(poll, 50);
  }

  async function reload() {
    pages = new Map();
    total = await call<number>("diff_count", { id, statuses: filters[filter] });
    scrollTop = 0;
    if (scroller) scroller.scrollTop = 0;
  }

  /** Rows come in pages of 200; at most 50 pages stay. */
  const asked = new Set<number>();
  function rowAt(index: number): Row | undefined {
    const page = Math.floor(index / PAGE);
    const rows = pages.get(page);
    if (rows) return rows[index % PAGE];
    if (!asked.has(page)) {
      asked.add(page);
      const statuses = filters[filter];
      void call<Row[]>("diff_rows", { id, statuses, start: page * PAGE, count: PAGE })
        .then((rows) => {
          if (statuses !== filters[filter]) return;
          const next = new Map(pages.size > 50 ? [] : pages);
          next.set(page, rows);
          pages = next;
        })
        .finally(() => asked.delete(page));
    }
    return undefined;
  }

  const number = (row: number | null, offset: number) => (row === null ? "—" : count(row - offset + 1));
  const single = (text: string | null) => (text ?? "").replace(/\r\n|\r|\n/g, " ⏎ ");
  const mark = { onlyLeft: "−", onlyRight: "+", changed: "≠", same: "=" };
  const markName = () => ({
    onlyLeft: L("Only in the first file"),
    onlyRight: L("Only in the second file"),
    changed: L("Changed row"),
    same: L("Same row"),
  });

  /** Shows a row in the table it came from, bringing its window forward. */
  async function reveal(row: Row) {
    if (row.leftRow === null) return;
    if (inTauri) {
      await emitTo(source, "reveal-row", row.leftRow);
      await (await WebviewWindow.getByLabel(source))?.setFocus();
    } else {
      new BroadcastChannel("comma-reveal").postMessage({ window: source, row: row.leftRow });
    }
  }

  async function openAsTable() {
    const names = [L("Only in %@", leftName), L("Only in %@", otherName), L("Changed Row"), L("Same Row")];
    const { token } = await call<{ token: number }>("diff_open", { id, statuses: filters[filter], names, statusTitle: L("Comparison") });
    newWindow(null, undefined, token);
  }

  onMount(() => {
    document.title = L("%@ and %@", leftName, otherName);
    void poll();
    const observer = new ResizeObserver(() => (viewHeight = scroller?.clientHeight ?? 400));
    if (scroller) observer.observe(scroller);
    return () => observer.disconnect();
  });

  onDestroy(() => {
    clearTimeout(timer);
    void call("diff_close", { id }).catch(() => {});
  });
</script>

<div class="diff">
  <div class="options">
    <label>
      {L("Match rows by:")}
      <select bind:value={key} disabled={busy} onchange={run}>
        <option value={-1}>{L("Row Number")}</option>
        <hr />
        {#each shared as column, index (column.left)}<option value={column.left}>{columnName(column, index)}</option>{/each}
      </select>
    </label>
    <label><input type="checkbox" bind:checked={ignoreCase} disabled={busy} onchange={run} />{L("Ignore case")}</label>
    <label><input type="checkbox" bind:checked={trim} disabled={busy} onchange={run} />{L("Ignore surrounding spaces")}</label>
    <span class="spacer"></span>
    <button disabled={busy || !info?.summary || total === 0} onclick={openAsTable}>{L("Open as Table")}</button>
  </div>
  <div class="filters" role="radiogroup" aria-label={L("Show")}>
    {#each filterLabels as label, index (index)}
      <button class:chosen={filter === index} disabled={busy} onclick={() => ((filter = index), void reload())}>{label}</button>
    {/each}
  </div>
  <div class="table" bind:this={scroller} onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}>
    <div class="header" style:grid-template-columns={widths}>
      <span aria-label={L("Difference")}></span>
      <span class="rows">{L("Rows")}</span>
      {#each columns as column, index (index)}<span title={columnTitle(column, index)}>{columnTitle(column, index)}</span>{/each}
    </div>
    <div class="body" style:height="{height}px">
      {#each visible as index (index)}
        {@const row = rowAt(index)}
        <div
          class="row {row?.status ?? ''}"
          style:grid-template-columns={widths}
          style:top="{(total * ROW <= MAX_HEIGHT ? index * ROW : scrollTop + (index - first) * ROW)}px"
          role="row"
          tabindex="-1"
          ondblclick={() => row && reveal(row)}
        >
          {#if row}
            <span class="mark" title={markName()[row.status]}>{mark[row.status]}</span>
            <span class="rows">{number(row.leftRow, info?.leftOffset ?? 1)} · {number(row.rightRow, info?.rightOffset ?? 1)}</span>
            {#each row.cells as cell, index (index)}
              {#if cell.changed && cell.left !== null && cell.right !== null}
                <span title="{single(cell.left)} → {single(cell.right)}" aria-label={L("Was %@, now %@", single(cell.left), single(cell.right))}>
                  <s>{single(cell.left)}</s>&nbsp; <b>{single(cell.right)}</b>
                </span>
              {:else}
                <span>{single(cell.right ?? cell.left)}</span>
              {/if}
            {/each}
          {/if}
        </div>
      {/each}
    </div>
  </div>
  <footer>
    <span class="status">{status}</span>
    {#if info?.comparing !== null && info?.comparing !== undefined}
      <button class="small" onclick={() => call("diff_cancel", { id })}>{L("Cancel")}</button>
    {/if}
  </footer>
</div>

<style>
  .diff {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 12px 16px 10px;
    box-sizing: border-box;
    gap: 8px;
  }

  .options {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .options label {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .spacer {
    flex: 1;
  }

  .filters {
    display: inline-flex;
    align-self: flex-start;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }

  .filters button {
    border: 0;
    border-radius: 0;
  }

  .filters .chosen {
    color: white;
    background: var(--accent);
  }

  .table {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    background: var(--grid-background);
  }

  .header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    font-weight: 600;
    background: var(--grid-header);
    border-bottom: 1px solid var(--grid-line);
  }

  .body {
    position: relative;
  }

  .row {
    position: absolute;
    left: 0;
    display: grid;
    height: 22px;
  }

  .header span,
  .row span {
    padding: 0 6px;
    line-height: 22px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    border-right: 1px solid var(--grid-line);
  }

  .rows {
    text-align: right;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .mark {
    font-weight: 700;
    text-align: center;
  }

  .onlyLeft {
    background: rgb(255 59 48 / 0.08);
  }

  .onlyLeft .mark {
    color: #ff3b30;
  }

  .onlyRight {
    background: rgb(52 199 89 / 0.08);
  }

  .onlyRight .mark {
    color: #34c759;
  }

  .changed .mark {
    color: #ff9500;
  }

  .same .mark,
  .same span {
    color: var(--muted);
  }

  s {
    color: #ff3b30;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 22px;
  }

  .status {
    flex: 1;
    color: var(--muted);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
</style>

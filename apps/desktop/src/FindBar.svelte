<!-- Find and replace over the table, as FindBarView in the macOS app: what is typed is
     counted and the first match from the active cell is selected. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { call, CoreFailure, type CellPosition, type CellRange, type EditResult, type SearchQuery, type Status } from "./lib/api";
  import type { Document } from "./lib/document.svelte";
  import { describe } from "./lib/errors";
  import { L, count } from "./lib/i18n";
  import { mac } from "./lib/keys";
  import type { Cell } from "./grid/selection";

  interface Props {
    document: Document;
    /** Whether the replace row shows. */
    replacing: boolean;
    /** The selection when the bar opened, to search in. */
    selection: CellRange | null;
    titles: string[];
    cursor: () => Cell | null;
    select: (cell: Cell) => void;
    close: () => void;
  }

  let { document: doc, replacing, selection, titles, cursor, select, close }: Props = $props();

  let field: HTMLInputElement;
  let text = $state("");
  let replacement = $state("");
  let caseSensitive = $state(false);
  let wholeCell = $state(false);
  let regex = $state(false);
  /** All columns, the selection, or one column. */
  let scope = $state<"all" | "selection" | number>("all");
  let result = $state("");
  let asked = 0;
  let timer = 0;

  const scopeLabel = (range: CellRange) =>
    L("In Selection (%@ × %@)", count(range.bottom - range.top + 1), count(range.right - range.left + 1));

  function query(): SearchQuery {
    return {
      text,
      caseSensitive,
      wholeCell,
      regex,
      column: typeof scope === "number" ? scope : null,
      range: scope === "selection" ? selection : null,
    };
  }

  function failed(error: unknown): string {
    const detail = error instanceof CoreFailure ? error.detail : error;
    if (detail && typeof detail === "object" && "invalidPattern" in detail) return L("Invalid regular expression");
    return describe(error).message;
  }

  /** The cell before the active one, so that a search starting there finds the active cell too. */
  function before(cell: Cell | null): CellPosition | null {
    if (!cell) return null;
    if (cell.column > 0) return { row: cell.row, column: cell.column - 1 };
    if (cell.row > 0) return { row: cell.row - 1, column: Math.max(0, titles.length - 1) };
    return null;
  }

  /** Counts the matches and, with `jump`, selects the first from the active cell. */
  async function update(jump: boolean) {
    const mine = ++asked;
    if (!text) {
      result = "";
      return;
    }
    result = L("Searching…");
    try {
      const matches = await call<number>("count_matches", { query: query() });
      if (mine !== asked) return;
      result = matches === 0 ? L("Not found") : L("Matches: %@", count(matches));
      if (jump && matches > 0) {
        const found = await call<CellPosition | null>("find", { query: query(), from: before(cursor()), forward: true });
        if (found && mine === asked) select(found);
      }
    } catch (error) {
      if (mine === asked) result = failed(error);
    }
  }

  function changed() {
    clearTimeout(timer);
    timer = window.setTimeout(() => void update(true), 200);
  }

  export async function next(forward: boolean) {
    if (!text) return;
    const at = cursor();
    try {
      const found = await call<CellPosition | null>("find", { query: query(), from: at, forward });
      if (found) select(found);
    } catch (error) {
      result = failed(error);
    }
  }

  async function replaceOne() {
    const at = cursor();
    if (!text || !at) return;
    try {
      await doc.runWith<{ status: Status; replaced: boolean }>("replace_in_cell", {
        row: at.row,
        column: at.column,
        query: query(),
        replacement,
      });
      await update(false);
      await next(true);
    } catch (error) {
      result = failed(error);
    }
  }

  async function replaceAll() {
    if (!text) return;
    try {
      const reply = await doc.runWith<{ status: Status; result: EditResult | null }>("replace_all", { query: query(), replacement });
      let replaced = reply.result?.count ?? null;
      if (replaced === null) {
        // A large table replaces in the background; the status bar shows how far.
        result = L("Replacing…");
        const ended = await doc.waitForJob();
        if (ended?.error) {
          result = failed(ended.error);
          return;
        }
        replaced = (ended?.ok as EditResult | undefined)?.count ?? 0;
      }
      result = L("Replaced: %@", count(replaced));
    } catch (error) {
      result = failed(error);
    }
  }

  export function focus() {
    field.focus();
    field.select();
  }

  function key(event: KeyboardEvent, inReplacement: boolean) {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (inReplacement) void replaceOne();
      else void next(!event.shiftKey);
    }
  }

  onMount(() => {
    if (selection && (selection.bottom > selection.top || selection.right > selection.left)) scope = "selection";
    focus();
  });
</script>

<div class="bar" role="search">
  <div class="row">
    <input
      bind:this={field}
      bind:value={text}
      oninput={changed}
      onkeydown={(event) => key(event, false)}
      placeholder={L("Find")}
      spellcheck="false"
      autocomplete="off"
    />
    <button class="small" onclick={() => next(false)} title={`${L("Find Previous")} (${mac ? "⇧⌘G" : "Shift+Ctrl+G"})`} aria-label={L("Find Previous")}>‹</button>
    <button class="small" onclick={() => next(true)} title={`${L("Find Next")} (${mac ? "⌘G" : "Ctrl+G"})`} aria-label={L("Find Next")}>›</button>
    <label><input type="checkbox" bind:checked={caseSensitive} onchange={() => update(false)} />{L("Match Case")}</label>
    <label><input type="checkbox" bind:checked={wholeCell} onchange={() => update(false)} />{L("Whole Cell")}</label>
    <label title={L("Use a regular expression; replacements can refer to groups as $1.")}>
      <input type="checkbox" bind:checked={regex} onchange={() => update(false)} />{L("Regex")}
    </label>
    <select bind:value={scope} onchange={() => update(false)} aria-label={L("Search in")}>
      <option value="all">{L("All Columns")}</option>
      {#if selection && (selection.bottom > selection.top || selection.right > selection.left)}
        <option value="selection">{scopeLabel(selection)}</option>
      {/if}
      <hr />
      {#each titles as title, column (column)}<option value={column}>{title}</option>{/each}
    </select>
    <span class="result" aria-live="polite">{result}</span>
    <button class="small" onclick={close}>{L("Done")}</button>
  </div>
  {#if replacing}
    <div class="row">
      <input bind:value={replacement} onkeydown={(event) => key(event, true)} placeholder={L("Replace with")} spellcheck="false" autocomplete="off" />
      <button class="small" onclick={replaceOne}>{L("Replace")}</button>
      <button class="small" onclick={replaceAll}>{L("Replace All")}</button>
    </div>
  {/if}
</div>

<style>
  .bar {
    padding: 5px 10px;
    background: var(--bar);
    border-bottom: 1px solid var(--border);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .row + .row {
    margin-top: 5px;
  }

  input:not([type]) {
    width: 220px;
    padding: 3px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
  }

  label {
    display: flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;
  }

  select {
    font: inherit;
    max-width: 200px;
  }

  .result {
    flex: 1;
    min-width: 80px;
    color: var(--muted);
    white-space: nowrap;
  }
</style>

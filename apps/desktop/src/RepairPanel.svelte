<!-- File repair (RepairView in the macOS app): broken rows by kind, each with the fixes the
     core offers and how the rows read after each. -->
<script lang="ts">
  import type { Problem, ProblemKind, RepairFix, RepairOption, RepairReport } from "./lib/api";
  import { L, count } from "./lib/i18n";

  interface Props {
    report: RepairReport | null;
    loading: boolean;
    selectedRow: number | null;
    rawText: string;
    options: RepairOption[];
    garbledExample: [string, string] | null;
    rowLabel: (row: number) => string;
    columnTitle: (column: number) => string;
    select: (problem: Problem) => void;
    apply: (row: number, fix: RepairFix) => void;
    fixAll: (kind: ProblemKind) => void;
    fixGarbled: () => void;
  }

  let { report, loading, selectedRow, rawText, options, garbledExample, rowLabel, columnTitle, select, apply, fixAll, fixGarbled }: Props =
    $props();

  const kindTitle = (kind: ProblemKind) =>
    ({
      unclosedQuote: L("Unclosed quote"),
      strayQuote: L("Stray quote"),
      missingFields: L("Missing fields"),
      extraFields: L("Extra fields"),
    })[kind];

  const kindDescription = (kind: ProblemKind) =>
    ({
      unclosedQuote: L("A quote opens a value and never closes, so everything up to the end of the file became one cell."),
      strayQuote: L("A quote merged several fields or rows into one value."),
      missingFields: L("Fewer fields than in other rows: a line break inside a value without quotes, or a blank line."),
      extraFields: L("More fields than in other rows: usually a delimiter inside a value without quotes."),
    })[kind];

  const fixAllHelp = (kind: ProblemKind) =>
    ({
      unclosedQuote: L("Keeps each quote as an ordinary character"),
      strayQuote: L("Keeps each quote as an ordinary character"),
      missingFields: L("Joins rows split by a line break, deletes blank lines, adds empty fields to the rest"),
      extraFields: L("Removes empty fields at the end or merges extra fields into the column where they fit"),
    })[kind];

  function fixTitle(fix: RepairFix): string {
    if (typeof fix === "object") return L("Merge extra fields into “%@”", columnTitle(fix.mergeFields.column));
    return {
      quoteAsText: L("Keep the quote as a character"),
      removeQuote: L("Remove the quote"),
      padFields: L("Add empty fields"),
      joinNext: L("Join with the next row"),
      dropEmptyFields: L("Remove empty fields at the end"),
      deleteRow: L("Delete the row"),
    }[fix];
  }

  const detail = (problem: Problem, expected: number) =>
    problem.lines > 1 ? L("Takes %@ lines of the file", count(problem.lines)) : L("Fields: %@ of %@", count(problem.fields), count(expected));

  function preview(option: RepairOption): string {
    if (option.rowCount === 0) return L("The row is deleted.");
    const lines = option.rows.map((row) => row.map((value) => (value === "" ? "∅" : value.replace(/\r\n|\r|\n/g, " ⏎ "))).join(" │ "));
    if (option.rowCount > option.rows.length) lines.push(L("…and %@ more rows", count(option.rowCount - option.rows.length)));
    return lines.join("\n");
  }

  const total = $derived(report ? report.counts.reduce((sum, item) => sum + item.count, 0) : 0);
  let list = $state<HTMLElement>();

  // The selected problem scrolls into view, as the panel follows the table.
  $effect(() => {
    if (selectedRow === null || !list) return;
    list.querySelector(`[data-row="${selectedRow}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<aside class="panel" aria-label={L("File Repair")}>
  <h2>{L("File Repair")}</h2>
  {#if report}
    {#if report.garbledSample > 0}
      <section class="group">
        <p class="title">⚠︎ {L("Garbled text: %@", count(report.garbledSample))}</p>
        <p class="note">{L("Some values look like text read in the wrong encoding and saved that way. Counted in the first rows.")}</p>
        {#if garbledExample}<p class="example">{garbledExample[0]} → {garbledExample[1]}</p>{/if}
        <button class="small" onclick={fixGarbled}>{L("Fix in the Whole File")}</button>
      </section>
    {/if}
    {#if report.counts.length === 0}
      {#if report.garbledSample === 0}<p class="note">✓ {L("No problems found")}</p>{/if}
    {:else}
      <p class="note">{L("Most rows have %@ fields.", count(report.expectedFields))}</p>
      {#each report.counts as item (item.kind)}
        <section class="group">
          <p class="title">⚠︎ {kindTitle(item.kind)}: {count(item.count)}</p>
          <p class="note">{kindDescription(item.kind)}</p>
          <button class="small" title={fixAllHelp(item.kind)} onclick={() => fixAll(item.kind)}>{L("Fix All")}</button>
        </section>
      {/each}
      <hr />
      <p class="note">{L("Select a row to see how to fix it.")}</p>
      <div class="problems" bind:this={list}>
        {#each report.problems as problem (problem.row)}
          <div class="problem" class:selected={problem.row === selectedRow} data-row={problem.row}>
            <button class="pick" onclick={() => select(problem)}>
              <span class="row">{rowLabel(problem.row)}</span>
              <span class="kind">{kindTitle(problem.kind)}</span>
              <span class="detail">{detail(problem, report.expectedFields)}</span>
            </button>
            {#if problem.row === selectedRow}
              <pre class="raw">{rawText}</pre>
              {#each options as option, index (index)}
                <div class="option">
                  {#if index === 0}<span class="recommended">{L("Recommended")}</span>{/if}
                  <button class="small" onclick={() => apply(problem.row, option.fix)}>{fixTitle(option.fix)}</button>
                  <pre class="preview">{preview(option)}</pre>
                </div>
              {/each}
            {/if}
          </div>
        {/each}
      </div>
      {#if total > report.problems.length}
        <p class="note">{L("Showing the first %@ problems.", count(report.problems.length))}</p>
      {/if}
    {/if}
  {:else if loading}
    <p class="note">…</p>
  {/if}
</aside>

<style>
  .panel {
    width: 320px;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 14px;
    background: var(--background);
    border-left: 1px solid var(--border);
  }

  h2 {
    margin: 0 0 12px;
    font-size: 14px;
  }

  .group {
    margin-bottom: 14px;
  }

  .title {
    margin: 0 0 4px;
    font-weight: 500;
  }

  .note {
    margin: 0 0 6px;
    font-size: 11px;
    color: var(--muted);
  }

  .example {
    margin: 0 0 6px;
    font: 11px ui-monospace, "Cascadia Mono", Consolas, monospace;
    overflow-wrap: anywhere;
  }

  hr {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 12px 0;
  }

  .problem {
    padding: 6px 8px;
    margin: 0 -8px 2px;
    border-radius: 6px;
  }

  .problem.selected {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .pick {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 8px;
    width: 100%;
    padding: 0;
    text-align: left;
    background: none;
    border: 0;
  }

  .row {
    font-weight: 500;
  }

  .kind,
  .detail {
    font-size: 11px;
    color: var(--muted);
    align-self: baseline;
  }

  .detail {
    grid-column: 1 / -1;
  }

  pre {
    margin: 6px 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: 11px ui-monospace, "Cascadia Mono", Consolas, monospace;
  }

  .raw {
    max-height: 6.5em;
    overflow: hidden;
    padding: 6px;
    background: color-mix(in srgb, var(--muted) 12%, transparent);
    border-radius: 4px;
    user-select: text;
  }

  .option {
    margin-top: 8px;
  }

  .recommended {
    display: block;
    font-size: 10px;
    font-weight: 500;
    color: var(--accent);
  }

  .preview {
    max-height: 9em;
    overflow: hidden;
    font-size: 10px;
    color: var(--muted);
  }
</style>

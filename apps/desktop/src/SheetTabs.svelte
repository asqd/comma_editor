<!--
  Tabs of the sheets of a workbook, in the order Excel shows them (SheetTabsView in the macOS
  app), and beside them the sheet's view: as Excel shows it, or as a CSV file. Hidden sheets
  are listed dimmed; sheets that are charts, not tables, cannot be chosen.
-->
<script lang="ts">
  import type { SheetInfo } from "./lib/api";
  import { L } from "./lib/i18n";

  let {
    sheets,
    selected,
    loading,
    csvMode,
    modeEnabled,
    select,
    setMode,
  }: {
    sheets: SheetInfo[];
    selected: number;
    /** A sheet being read, shown with an ellipsis. */
    loading: number | null;
    csvMode: boolean;
    modeEnabled: boolean;
    select: (index: number) => void;
    setMode: (csv: boolean) => void;
  } = $props();

  let tabs = $state<HTMLElement>();

  // The chosen tab stays in view.
  $effect(() => {
    void selected;
    tabs?.querySelector<HTMLElement>('[aria-checked="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  const tip = (sheet: SheetInfo) =>
    !sheet.isTable ? L("A chart sheet: it has no cells to show") : sheet.visibility !== "visible" ? L("A hidden sheet") : undefined;
</script>

<div class="sheets">
  <div class="tabs" role="radiogroup" aria-label={L("Sheets")} bind:this={tabs}>
    {#each sheets as sheet, index (index)}
      <button
        role="radio"
        aria-checked={index === selected}
        class:hidden={sheet.isTable && sheet.visibility !== "visible"}
        disabled={!sheet.isTable}
        title={tip(sheet)}
        onclick={() => select(index)}>{index === loading ? `${sheet.name} …` : sheet.name}</button
      >
    {/each}
  </div>
  <div class="mode" role="radiogroup" aria-label={L("Sheet view")}>
    <button
      role="radio"
      aria-checked={!csvMode}
      disabled={!modeEnabled}
      title={L("As Excel shows the sheet: columns lettered A, B, C")}
      onclick={() => csvMode && setMode(false)}>{L("Table")}</button
    >
    <button
      role="radio"
      aria-checked={csvMode}
      disabled={!modeEnabled}
      title={L("As a CSV file: the first row names the columns")}
      onclick={() => !csvMode && setMode(true)}>CSV</button
    >
  </div>
</div>

<style>
  .sheets {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    min-height: 28px;
    background: var(--bar);
    border-top: 1px solid var(--border);
  }

  .tabs {
    display: flex;
    flex: 1;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tabs button,
  .mode button {
    flex: none;
    padding: 2px 10px;
    font-size: 12px;
    color: var(--text);
    background: none;
    border: 1px solid transparent;
    border-radius: 5px;
    white-space: nowrap;
  }

  .tabs button:hover:not(:disabled) {
    background: rgb(128 128 128 / 0.14);
  }

  .tabs button[aria-checked="true"] {
    background: var(--background);
    border-color: var(--border);
  }

  .tabs button.hidden,
  .tabs button:disabled {
    color: var(--muted);
  }

  .tabs button.hidden {
    font-style: italic;
  }

  .mode {
    display: flex;
    flex: none;
    padding: 1px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .mode button {
    padding: 1px 8px;
    border: 0;
  }

  .mode button[aria-checked="true"] {
    color: #fff;
    background: var(--accent);
  }

  .mode button:disabled {
    opacity: 0.5;
  }
</style>

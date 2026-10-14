<!-- The active row as a form, one field per column, for tables too wide to read across
     (RecordCardView in the macOS app). Follows the table's cursor. -->
<script lang="ts">
  import { L } from "./lib/i18n";
  import { shortcutLabel } from "./lib/keys";

  export interface Field {
    column: number;
    name: string;
    value: string;
  }

  interface Props {
    /** The visible row shown, or `null` without rows. */
    row: number | null;
    title: string;
    position: string;
    fields: Field[];
    activeColumn: number;
    canGoBack: boolean;
    canGoForward: boolean;
    edit: (row: number, column: number, value: string) => void;
    selectColumn: (row: number, column: number) => void;
    previous: () => void;
    next: () => void;
  }

  let { row, title, position, fields, activeColumn, canGoBack, canGoForward, edit, selectColumn, previous, next }: Props = $props();

  let query = $state("");
  let hidesEmpty = $state(false);
  /** What is typed into a field and not yet written, by column, for the row shown. */
  let drafts = $state<Record<number, string>>({});
  let draftsFor: number | null = null;
  let list = $state<HTMLElement>();

  // A new row starts with fresh drafts.
  $effect(() => {
    if (row !== draftsFor) {
      draftsFor = row;
      drafts = {};
    }
  });

  const shown = $derived(
    fields.filter((field) => {
      const needle = query.trim().toLowerCase();
      return (
        (!hidesEmpty || field.value !== "") &&
        (needle === "" || field.name.toLowerCase().includes(needle) || field.value.toLowerCase().includes(needle))
      );
    }),
  );

  // The active column's field scrolls into view.
  $effect(() => {
    list?.querySelector(`[data-column="${activeColumn}"]`)?.scrollIntoView({ block: "nearest" });
  });

  /** Writes what was typed into a field, if it differs from the value. */
  function commit(field: Field) {
    const draft = drafts[field.column];
    if (row === null || draft === undefined || draft === field.value) return;
    delete drafts[field.column];
    edit(row, field.column, draft);
  }

  function key(event: KeyboardEvent, field: Field) {
    if (event.key === "Enter" && !event.shiftKey && !event.altKey) {
      event.preventDefault();
      commit(field);
    } else if (event.key === "Escape") {
      event.preventDefault();
      delete drafts[field.column];
      (event.currentTarget as HTMLElement).blur();
    }
  }

  /** Fields grow with their text, up to eight lines. */
  function fit(element: HTMLTextAreaElement) {
    const resize = () => {
      element.style.height = "auto";
      element.style.height = `${Math.min(element.scrollHeight, 8 * 18 + 8)}px`;
    };
    resize();
    element.addEventListener("input", resize);
    return { update: resize, destroy: () => element.removeEventListener("input", resize) };
  }
</script>

<aside class="panel" aria-label={L("Card")}>
  <header>
    <div class="heading">
      <div>
        <h2>{title}</h2>
        <p class="position">{position}</p>
      </div>
      <div class="navigation">
        <button class="small" disabled={!canGoBack} onclick={previous} aria-label={L("Previous record")} title="{L('Previous record')} ({shortcutLabel({ key: '[', command: true })})">˄</button>
        <button class="small" disabled={!canGoForward} onclick={next} aria-label={L("Next record")} title="{L('Next record')} ({shortcutLabel({ key: ']', command: true })})">˅</button>
      </div>
    </div>
    <div class="search">
      <input bind:value={query} placeholder={L("Find a field")} spellcheck="false" autocomplete="off" />
      <label><input type="checkbox" bind:checked={hidesEmpty} />{L("Hide empty")}</label>
    </div>
  </header>
  {#if row === null}
    <p class="none">{L("No rows")}</p>
  {:else}
    <div class="fields" bind:this={list}>
      {#each shown as field (field.column)}
        <div class="field" class:active={field.column === activeColumn} data-column={field.column}>
          <button class="name" title={field.name} onclick={() => selectColumn(row, field.column)}>{field.name}</button>
          <textarea
            rows="1"
            use:fit
            aria-label={field.name}
            placeholder={L("(empty)")}
            spellcheck="false"
            value={drafts[field.column] ?? field.value}
            oninput={(event) => (drafts[field.column] = event.currentTarget.value)}
            onfocus={() => selectColumn(row, field.column)}
            onblur={() => commit(field)}
            onkeydown={(event) => key(event, field)}
          ></textarea>
        </div>
      {/each}
    </div>
  {/if}
</aside>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    width: 360px;
    box-sizing: border-box;
    background: var(--background);
    border-left: 1px solid var(--border);
  }

  header {
    padding: 14px;
    border-bottom: 1px solid var(--border);
  }

  .heading {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  h2 {
    margin: 0;
    font-size: 14px;
  }

  .position {
    margin: 2px 0 0;
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .navigation {
    display: flex;
    gap: 2px;
  }

  .search {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
  }

  .search input:not([type]) {
    flex: 1;
    padding: 3px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
  }

  label {
    display: flex;
    gap: 4px;
    align-items: center;
    white-space: nowrap;
  }

  .none {
    padding: 14px;
    color: var(--muted);
  }

  .fields {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }

  .field {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 3px 12px;
  }

  .field.active {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  .name {
    width: 110px;
    flex: none;
    padding: 0;
    overflow: hidden;
    text-align: right;
    text-overflow: ellipsis;
    color: var(--muted);
    background: none;
    border: 0;
  }

  .active .name {
    color: var(--accent);
  }

  textarea {
    flex: 1;
    box-sizing: border-box;
    padding: 3px 5px;
    font: inherit;
    line-height: 18px;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid color-mix(in srgb, var(--muted) 25%, transparent);
    border-radius: 4px;
    resize: none;
    overflow-y: auto;
  }

  textarea:focus {
    border-color: var(--accent);
    outline: none;
  }
</style>

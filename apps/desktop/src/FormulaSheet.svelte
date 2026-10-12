<!-- Data ▸ Computed Column (FormulaView in the macOS app): a formula over the columns of each
     row, with a live preview, into a new column or over the active one. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { call, CoreFailure } from "./lib/api";
  import { describeFormula, functions, type FormulaProblem } from "./lib/formula";
  import { L, count } from "./lib/i18n";

  interface Props {
    columns: string[];
    activeColumnName: string;
    apply: (choice: { formula: string; target: "new" | "active"; name: string }) => void;
    cancel: () => void;
  }

  let { columns, activeColumnName, apply, cancel }: Props = $props();
  const PREVIEW_ROWS = 8;

  let dialog: HTMLDialogElement;
  let field: HTMLTextAreaElement;
  let formula = $state("");
  let name = $state(L("Result"));
  let target = $state<"new" | "active">("new");
  let preview = $state.raw<{ label: string; value: string; error: boolean }[]>([]);
  let problem = $state("");
  let asked = 0;
  let timer = 0;

  const canApply = $derived(formula.trim() !== "" && problem === "" && (target === "active" || name.trim() !== ""));

  async function refresh() {
    const mine = ++asked;
    if (formula.trim() === "") {
      preview = [];
      problem = "";
      return;
    }
    try {
      const result = await call<{ outcomes: { value: string | null; problem: FormulaProblem | null }[]; numbers: number[] }>("preview_formula", {
        formula,
        count: PREVIEW_ROWS,
      });
      if (mine !== asked) return;
      problem = "";
      preview = result.outcomes.map((outcome, index) => ({
        label: L("Row %@", count(result.numbers[index] ?? index + 1)),
        value: outcome.value ?? (outcome.problem ? describeFormula(outcome.problem) : ""),
        error: outcome.problem !== null,
      }));
    } catch (error) {
      if (mine !== asked) return;
      preview = [];
      const detail = error instanceof CoreFailure ? (error.detail as { formula?: { problem: FormulaProblem } }) : null;
      problem = detail?.formula ? describeFormula(detail.formula.problem) : String(error);
    }
  }

  function changed() {
    clearTimeout(timer);
    timer = window.setTimeout(() => void refresh(), 150);
  }

  /** Inserts a column or a function where typing goes on. */
  function insert(text: string) {
    const separator = formula === "" || formula.endsWith(" ") || formula.endsWith("(") ? "" : " ";
    formula += separator + text;
    changed();
    field.focus();
  }

  onMount(() => {
    dialog.showModal();
    field.focus();
  });
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    cancel();
  }}
>
  <h2>{L("Computed Column")}</h2>
  <textarea bind:this={field} bind:value={formula} oninput={changed} rows="2" placeholder={"{price} * {qty}"} aria-label={L("Formula")} spellcheck="false"></textarea>
  <div class="inserts">
    <select aria-label={L("Column")} onchange={(event) => { const value = event.currentTarget.value; event.currentTarget.value = ""; if (value) insert(`{${value}}`); }}>
      <option value="">{L("Column")}</option>
      {#each columns as column, index (index)}<option value={column}>{column}</option>{/each}
    </select>
    <select aria-label={L("Function")} onchange={(event) => { const value = event.currentTarget.value; event.currentTarget.value = ""; if (value) insert(`${value}(`); }}>
      <option value="">{L("Function")}</option>
      {#each functions() as item (item.name)}<option value={item.name}>{item.signature} — {item.summary}</option>{/each}
    </select>
  </div>
  {#if problem}
    <p class="problem">⚠︎ {problem}</p>
  {:else}
    <p class="note">
      {L("Columns go in braces: {price}. Text goes in quotes: \"kg\". & joins text; +, -, *, / and % count; =, <>, <, > compare; AND, OR, NOT combine.")}
    </p>
  {/if}

  <div class="targets" role="radiogroup" aria-label={L("Result")}>
    <button class:chosen={target === "new"} onclick={() => (target = "new")}>{L("New column")}</button>
    <button class:chosen={target === "active"} onclick={() => (target = "active")}>{L("Replace “%@”", activeColumnName)}</button>
  </div>
  {#if target === "new"}
    <label class="name">{L("Name:")} <input bind:value={name} aria-label={L("Column name")} /></label>
    <p class="note">{L("The column is added to the right of “%@”.", activeColumnName)}</p>
  {:else}
    <p class="note">{L("Values of the shown rows are replaced. The formula can use the column itself.")}</p>
  {/if}

  <h3>{L("Preview")}</h3>
  <div class="preview">
    {#each preview as row (row.label)}
      <span class="label">{row.label}</span>
      <span class:error={row.error} class:empty={row.value === ""}>{row.value === "" ? "∅" : row.value}</span>
    {/each}
  </div>

  <footer>
    <button onclick={cancel}>{L("Cancel")}</button>
    <button class="default" disabled={!canApply} onclick={() => apply({ formula, target, name: name.trim() })}>
      {target === "new" ? L("Add Column") : L("Replace Values")}
    </button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 32px));
    padding: 20px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  h2 {
    margin: 0 0 12px;
    font-size: 14px;
  }

  h3 {
    margin: 12px 0 4px;
    font-size: 12px;
  }

  textarea {
    box-sizing: border-box;
    width: 100%;
    padding: 6px;
    font: 13px ui-monospace, "Cascadia Mono", Consolas, monospace;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
    resize: vertical;
  }

  .inserts {
    display: flex;
    gap: 8px;
    margin: 6px 0;
  }

  select {
    font: inherit;
    max-width: 260px;
  }

  .note {
    margin: 4px 0 8px;
    font-size: 11px;
    color: var(--muted);
  }

  .problem {
    margin: 4px 0 8px;
  }

  .targets {
    display: inline-flex;
    margin: 4px 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }

  .targets button {
    border: 0;
    border-radius: 0;
  }

  .targets .chosen {
    color: white;
    background: var(--accent);
  }

  .name {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .name input {
    flex: 1;
    padding: 3px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
  }

  .preview {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 12px;
    min-height: 150px;
    align-content: start;
    padding: 8px;
    background: color-mix(in srgb, var(--muted) 8%, transparent);
    border-radius: 6px;
  }

  .preview span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    user-select: text;
  }

  .label {
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .error {
    color: #e67e00;
  }

  .empty {
    color: var(--muted);
  }

  footer {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 16px;
  }

  .default {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }
</style>

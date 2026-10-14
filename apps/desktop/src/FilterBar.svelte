<!-- Filter conditions, all of which a row meets to show, as FilterBarView in the macOS app.
     Typing applies after a pause, choices at once; removing the last condition empties it. -->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { FilterOp, FilterRule } from "./lib/api";
  import { L } from "./lib/i18n";

  interface Props {
    /** The filters shown when the bar opens. */
    filters: FilterRule[];
    titles: string[];
    /** Why the filters cannot apply, such as an invalid regular expression. */
    problem: string;
    apply: (filters: FilterRule[]) => void;
    close: () => void;
  }

  let { filters, titles, problem, apply, close }: Props = $props();

  interface Condition {
    /** `null` is any column. */
    column: number | null;
    op: FilterOp;
    value: string;
    caseSensitive: boolean;
  }

  const operators: [FilterOp, string][] = [
    ["contains", L("contains")],
    ["notContains", L("does not contain")],
    ["equals", L("is")],
    ["notEquals", L("is not")],
    ["startsWith", L("begins with")],
    ["endsWith", L("ends with")],
    ["matches", L("matches regex")],
    ["greaterThan", L("is greater than")],
    ["lessThan", L("is less than")],
    ["isEmpty", L("is empty")],
    ["isNotEmpty", L("is not empty")],
  ];
  const valueless = (op: FilterOp) => op === "isEmpty" || op === "isNotEmpty";
  const blank = (): Condition => ({ column: null, op: "contains", value: "", caseSensitive: false });

  let conditions = $state<Condition[]>(untrack(() => (filters.length > 0 ? filters.map((rule) => ({ ...rule })) : [blank()])));
  let timer = 0;

  /** The conditions that filter: those with a value, or that need none. */
  function rules(): FilterRule[] {
    return conditions
      .filter((condition) => condition.value !== "" || valueless(condition.op))
      .map((condition) => ({ ...condition }));
  }

  function now() {
    clearTimeout(timer);
    apply(rules());
  }

  function later() {
    clearTimeout(timer);
    timer = window.setTimeout(() => apply(rules()), 250);
  }

  function add(after: number) {
    conditions.splice(after + 1, 0, blank());
  }

  function remove(index: number) {
    if (conditions.length === 1) conditions[0] = blank();
    else conditions.splice(index, 1);
    now();
  }

  let bar: HTMLElement;

  /**
   * A condition for a column, from its title's filter icon (FilterBarView.beginRule): the
   * first empty condition takes the column, or a new one; its value gets the focus.
   */
  export async function beginRule(column: number) {
    let index = conditions.findIndex((condition) => condition.value === "" && !valueless(condition.op));
    if (index < 0) {
      conditions.push({ ...blank(), column });
      index = conditions.length - 1;
    } else {
      conditions[index].column = column;
    }
    await tick();
    bar.querySelectorAll<HTMLInputElement>("input")[index]?.focus();
  }

  /** Adds a condition from a cell's menu, replacing an empty one. */
  export function addRule(rule: FilterRule) {
    const empty = conditions.findIndex((condition) => condition.value === "" && !valueless(condition.op));
    if (empty >= 0) conditions[empty] = { ...rule };
    else conditions.push({ ...rule });
    now();
  }
</script>

<div class="bar" bind:this={bar}>
  {#each conditions as condition, index (index)}
    <div class="row">
      <select bind:value={condition.column} onchange={now} aria-label={L("Column")}>
        <option value={null}>{L("Any Column")}</option>
        <hr />
        {#each titles as title, column (column)}<option value={column}>{title}</option>{/each}
      </select>
      <select bind:value={condition.op} onchange={now} aria-label={L("Condition")}>
        {#each operators as [op, label] (op)}<option value={op}>{label}</option>{/each}
      </select>
      {#if !valueless(condition.op)}
        <input bind:value={condition.value} oninput={later} placeholder={L("Value")} aria-label={L("Value")} spellcheck="false" autocomplete="off" />
      {/if}
      <button class="small" onclick={() => remove(index)} title={L("Remove Condition")} aria-label={L("Remove Condition")}>−</button>
      <button class="small" onclick={() => add(index)} title={L("Add Condition")} aria-label={L("Add Condition")}>+</button>
      {#if index === 0}
        <span class="problem">{problem}</span>
        <button class="small" onclick={close}>{L("Done")}</button>
      {/if}
    </div>
  {/each}
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
    gap: 6px;
  }

  .row + .row {
    margin-top: 5px;
  }

  select {
    font: inherit;
    max-width: 200px;
  }

  input {
    width: 200px;
    padding: 3px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
  }

  .problem {
    flex: 1;
    color: var(--error);
  }
</style>

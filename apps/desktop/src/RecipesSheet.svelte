<!-- Recipes ▸ Manage Recipes (RecipesView in the macOS app): names, steps, deleting them,
     exporting a recipe for Python and applying it to this table. -->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { describeStep, type Recipe } from "./lib/recipes";
  import { L } from "./lib/i18n";

  interface Props {
    recipes: Recipe[];
    canApply: boolean;
    rename: (file: string, name: string) => Promise<void>;
    removeStep: (file: string, index: number) => Promise<void>;
    remove: (file: string) => Promise<void>;
    exportPython: (recipe: Recipe) => void;
    apply: (recipe: Recipe) => void;
    close: () => void;
  }

  let { recipes, canApply, rename, removeStep, remove, exportPython, apply, close }: Props = $props();
  let dialog: HTMLDialogElement;
  let chosen = $state<string | null>(untrack(() => recipes[0]?.file ?? null));
  let step = $state<number | null>(null);
  let name = $state("");
  const recipe = $derived(recipes.find((item) => item.file === chosen) ?? null);

  // The chosen recipe's name in the field.
  $effect(() => {
    name = recipe?.name ?? "";
  });

  async function commitName() {
    const trimmed = name.trim();
    if (recipe && trimmed !== "" && trimmed !== recipe.name) await rename(recipe.file, trimmed);
  }

  onMount(() => dialog.showModal());
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    void commitName().then(close);
  }}
>
  <h2>{L("Recipes")}</h2>
  {#if recipes.length === 0}
    <p class="empty">
      {L("No recipes yet. Choose Recipes ▸ Start Recording, clean up a table as usual and stop recording: the steps can then be repeated on the next file.")}
    </p>
  {:else}
    <div class="body">
      <ul class="list" role="listbox">
        {#each recipes as item (item.file)}
          <li>
            <button class:chosen={item.file === chosen} onclick={() => void commitName().then(() => ((chosen = item.file), (step = null)))}>
              {item.name || L("Untitled Recipe")}
            </button>
          </li>
        {/each}
      </ul>
      {#if recipe}
        <div class="details">
          <input bind:value={name} placeholder={L("Name")} onkeydown={(event) => event.key === "Enter" && commitName()} onblur={commitName} />
          <ol class="steps">
            {#each recipe.steps as item, index (index)}
              <li>
                <button class:chosen={index === step} onclick={() => (step = index)}>{index + 1}. {describeStep(item)}</button>
              </li>
            {/each}
          </ol>
          <div class="row">
            <button disabled={step === null} onclick={() => step !== null && removeStep(recipe.file, step).then(() => (step = null))}>{L("Delete Step")}</button>
            <button onclick={() => remove(recipe.file).then(() => ((chosen = recipes.find((item) => item.file !== recipe.file)?.file ?? null), (step = null)))}>
              {L("Delete Recipe")}
            </button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
  <footer>
    <button disabled={!recipe} onclick={() => recipe && commitName().then(() => exportPython(recipe))}>{L("Export for Python…")}</button>
    <span class="spacer"></span>
    <button onclick={() => void commitName().then(close)}>{L("Done")}</button>
    <button class="default" disabled={!recipe || recipe.steps.length === 0 || !canApply} onclick={() => recipe && commitName().then(() => apply(recipe))}>
      {L("Apply to This Table")}
    </button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(680px, calc(100vw - 32px));
    height: min(420px, calc(100vh - 32px));
    padding: 20px;
    box-sizing: border-box;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  dialog[open] {
    display: flex;
    flex-direction: column;
  }

  h2 {
    margin: 0 0 12px;
    font-size: 14px;
  }

  .empty {
    flex: 1;
    color: var(--muted);
  }

  .body {
    flex: 1;
    display: flex;
    gap: 12px;
    min-height: 0;
  }

  ul,
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  .list {
    width: 190px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  li button {
    width: 100%;
    padding: 4px 8px;
    text-align: left;
    background: none;
    border: 0;
    border-radius: 0;
  }

  li button.chosen {
    color: white;
    background: var(--accent);
  }

  .details {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .details input {
    padding: 4px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 5px;
  }

  .steps {
    flex: 1;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .steps button {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .row,
  footer {
    display: flex;
    gap: 8px;
  }

  footer {
    margin-top: 14px;
  }

  .spacer {
    flex: 1;
  }

  .default {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }
</style>

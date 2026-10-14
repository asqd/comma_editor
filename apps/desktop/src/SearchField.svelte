<!-- The search field at the right of the toolbar (NSSearchToolbarItem in the macOS app): it
     searches as one types, Return finds the next match and Shift-Return the previous one,
     Escape clears it and gives the table the keys back. -->
<script lang="ts">
  import { L } from "./lib/i18n";

  interface Props {
    value: string;
    disabled: boolean;
    changed: (text: string) => void;
    find: (forward: boolean) => void;
    cancel: () => void;
  }

  let { value = $bindable(), disabled, changed, find, cancel }: Props = $props();
  let field: HTMLInputElement;

  export function focus() {
    field.focus();
    field.select();
  }

  function key(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      find(!event.shiftKey);
    } else if (event.key === "Escape") {
      event.preventDefault();
      value = "";
      changed("");
      cancel();
    }
  }
</script>

<div class="search" role="search">
  <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="6.5" cy="6.5" r="4.5" /><path d="M10 10l4 4" /></svg>
  <input
    bind:this={field}
    bind:value
    {disabled}
    type="search"
    placeholder={L("Find")}
    aria-label={L("Find")}
    spellcheck="false"
    autocomplete="off"
    oninput={() => changed(value)}
    onkeydown={key}
  />
</div>

<style>
  .search {
    position: relative;
    flex: 0 1 200px;
    min-width: 110px;
    margin-left: auto;
  }

  svg {
    position: absolute;
    left: 7px;
    top: 50%;
    width: 13px;
    height: 13px;
    transform: translateY(-50%);
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.6;
    stroke-linecap: round;
    pointer-events: none;
  }

  input {
    box-sizing: border-box;
    width: 100%;
    padding: 4px 8px 4px 25px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
</style>

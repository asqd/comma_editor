<!-- A menu at the pointer, for the cells and the column titles. Escape, a click elsewhere or
     a choice closes it. -->
<script lang="ts">
  import { onMount } from "svelte";

  export interface MenuItem {
    label: string;
    shortcut?: string;
    disabled?: boolean;
    checked?: boolean;
    run?: () => void;
  }

  interface Props {
    x: number;
    y: number;
    /** Items; `null` separates groups. */
    items: (MenuItem | null)[];
    close: () => void;
  }

  let { x, y, items, close }: Props = $props();
  let menu: HTMLElement;
  let left = $state(0);
  let top = $state(0);

  function choose(item: MenuItem) {
    if (item.disabled) return;
    close();
    item.run?.();
  }

  onMount(() => {
    // Inside the window, as menus open.
    const bounds = menu.getBoundingClientRect();
    left = Math.max(4, Math.min(x, innerWidth - bounds.width - 4));
    top = Math.max(4, Math.min(y, innerHeight - bounds.height - 4));
    menu.focus();
    const outside = (event: PointerEvent) => {
      if (!menu.contains(event.target as Node)) close();
    };
    addEventListener("pointerdown", outside, true);
    return () => removeEventListener("pointerdown", outside, true);
  });
</script>

<div
  class="menu"
  role="menu"
  tabindex="-1"
  bind:this={menu}
  style:left="{left}px"
  style:top="{top}px"
  onkeydown={(event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    }
  }}
>
  {#each items as item, index (index)}
    {#if item === null}
      <div class="separator" role="separator"></div>
    {:else}
      <button role="menuitem" class:checked={item.checked} disabled={item.disabled} onclick={() => choose(item)}>
        <span class="check">{item.checked ? "✓" : ""}</span>
        <span class="label">{item.label}</span>
        <span class="shortcut">{item.shortcut ?? ""}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 10;
    min-width: 200px;
    padding: 4px;
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 7px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    outline: none;
  }

  button {
    display: grid;
    grid-template-columns: 14px 1fr auto;
    gap: 6px;
    width: 100%;
    padding: 3px 8px;
    text-align: left;
    background: none;
    border: 0;
    border-radius: 4px;
  }

  button:not(:disabled):hover {
    color: white;
    background: var(--accent);
  }

  button:not(:disabled):hover .shortcut {
    color: rgb(255 255 255 / 0.8);
  }

  .shortcut {
    padding-left: 16px;
    color: var(--muted);
  }

  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>

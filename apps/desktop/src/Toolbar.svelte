<!-- The window's toolbar (DocumentToolbar.swift): buttons for frequent commands and menus of
     others, from the same commands as the menu bar. -->
<script lang="ts">
  import Icon, { type IconName } from "./Icon.svelte";
  import type { MenuItem } from "./ContextMenu.svelte";

  export interface ToolbarItem {
    id: string;
    label: string;
    tooltip: string;
    icon: IconName;
    enabled: boolean;
    /** Drawn filled, as Sort and Filter are while they apply. */
    active?: boolean;
    /** A button runs this; a menu item shows `menu` below it. */
    run?: () => void;
    menu?: () => (MenuItem | null)[];
  }

  interface Props {
    items: (ToolbarItem | null)[];
    openMenu: (x: number, y: number, items: (MenuItem | null)[]) => void;
  }

  let { items, openMenu }: Props = $props();

  function press(item: ToolbarItem, event: MouseEvent) {
    if (item.menu) {
      const bounds = (event.currentTarget as HTMLElement).getBoundingClientRect();
      openMenu(bounds.left, bounds.bottom + 2, item.menu());
    } else {
      item.run?.();
    }
  }
</script>

<div class="items" role="toolbar">
  {#each items as item, index (item?.id ?? `space-${index}`)}
    {#if item === null}
      <span class="space"></span>
    {:else}
      <button
        class="tool"
        class:menu={!!item.menu}
        disabled={!item.enabled}
        title={item.tooltip}
        aria-label={item.label}
        onmousedown={(event) => event.preventDefault()}
        onclick={(event) => press(item, event)}
      >
        <Icon name={item.icon} filled={item.active} />
        {#if item.menu}<span class="indicator">▾</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .items {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .tool {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px 7px;
    color: var(--text);
    line-height: 0;
  }

  .tool:disabled {
    color: var(--muted);
    opacity: 0.55;
  }

  .indicator {
    font-size: 9px;
    line-height: 1;
    color: var(--muted);
  }

  .space {
    width: 10px;
  }
</style>

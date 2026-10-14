<!-- The toolbar's icons, drawn after the SF Symbols the macOS app uses. -->
<script lang="ts">
  export type IconName = "insertRow" | "insertColumn" | "trash" | "sort" | "filter" | "data" | "export" | "profile" | "card" | "repair";

  let { name, filled = false }: { name: IconName; filled?: boolean } = $props();
</script>

<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">
  {#if name === "insertRow"}
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <path d="M2.6 8.5h10.8v3.4a1 1 0 0 1-1 1H3.6a1 1 0 0 1-1-1z" fill="currentColor" stroke="none" />
  {:else if name === "insertColumn"}
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <path d="M8.5 3.1h3.4a1 1 0 0 1 1 1v7.8a1 1 0 0 1-1 1H8.5z" fill="currentColor" stroke="none" />
  {:else if name === "trash"}
    <path d="M2.5 4h11M6 4V2.6h4V4M3.8 4l.7 9.4h7l.7-9.4M6.6 6.5v5M9.4 6.5v5" />
  {:else if name === "sort"}
    <circle cx="8" cy="8" r="6.3" fill={filled ? "currentColor" : "none"} />
    <path d="M6 10.5V5.2M4.4 6.7 6 5.1l1.6 1.6M10 5.5v5.3M8.4 9.3 10 10.9l1.6-1.6" stroke={filled ? "var(--bar)" : "currentColor"} />
  {:else if name === "filter"}
    <circle cx="8" cy="8" r="6.3" fill={filled ? "currentColor" : "none"} />
    <path d="M4.8 6h6.4M5.8 8h4.4M6.9 10h2.2" stroke={filled ? "var(--bar)" : "currentColor"} />
  {:else if name === "data"}
    <path d="M3 13 10 6M9.2 5.2l1.6 1.6M12 2.2v2M11 3.2h2M5 2.6v1.6M4.2 3.4h1.6M13 8.6v1.6M12.2 9.4h1.6" />
  {:else if name === "export"}
    <path d="M5.5 5.5H4a1 1 0 0 0-1 1V13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V6.5a1 1 0 0 0-1-1h-1.5M8 10V2M5.5 4.5 8 2l2.5 2.5" />
  {:else if name === "profile"}
    <path d="M2.5 13.5h11M4 11V8M6.7 11V5M9.3 11V7M12 11V3.5" />
  {:else if name === "card"}
    <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
    <path d="M4.5 6h.1M7 6h4.5M4.5 8.5h.1M7 8.5h4.5M4.5 11h.1M7 11h4.5" />
  {:else if name === "repair"}
    <path d="m3.6 9.4 5.8-5.8a2.3 2.3 0 0 1 3.2 3.2l-5.8 5.8a2.3 2.3 0 0 1-3.2-3.2zM6.5 6.5l3 3M7.6 8.4h.1M8.5 7.5h.1" />
  {/if}
</svg>

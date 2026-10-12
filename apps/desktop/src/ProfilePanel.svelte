<!-- Type, statistics, distribution, problems and frequent values of the active column
     (ColumnProfileView in the macOS app). -->
<script lang="ts">
  import type { ColumnProfile } from "./lib/api";
  import { kindName } from "./lib/formats";
  import { L, count, locale } from "./lib/i18n";

  interface Props {
    name: string;
    profile: ColumnProfile | null;
    loading: boolean;
    showIssue: (issue: "mismatch" | "padding") => void;
    trim: () => void;
    filterValue: (value: string) => void;
  }

  let { name, profile, loading, showIssue, trim, filterValue }: Props = $props();

  const decimal = new Intl.NumberFormat(locale, { maximumFractionDigits: 6 });
  const share = new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 });

  const percent = (part: number, total: number) => (total > 0 && part > 0 ? ` (${share.format(part / total)})` : "");

  const statistics = $derived.by((): [string, string][] => {
    if (!profile) return [];
    const rows: [string, string][] = [
      [L("Values"), count(profile.total)],
      [L("Empty"), count(profile.empty) + percent(profile.empty, profile.total)],
      [L("Unique"), count(profile.unique)],
    ];
    if (profile.kind === "integer" || profile.kind === "decimal") {
      rows.push(
        [L("Minimum"), profile.min ?? "—"],
        [L("Maximum"), profile.max ?? "—"],
        [L("Average"), profile.mean === null ? "—" : decimal.format(profile.mean)],
        [L("Median"), profile.median === null ? "—" : decimal.format(profile.median)],
      );
    } else if (profile.kind === "date") {
      rows.push([L("Earliest"), profile.min ?? "—"], [L("Latest"), profile.max ?? "—"]);
    } else {
      const length = profile.minLength === profile.maxLength ? count(profile.minLength) : `${count(profile.minLength)}–${count(profile.maxLength)}`;
      rows.push([L("Length"), length]);
    }
    return rows;
  });
</script>

<aside class="panel" aria-label={L("Profile")}>
  <h2>{name}</h2>
  {#if profile}
    <span class="kind">{kindName(profile.kind)}</span>
    <dl>
      {#each statistics as [label, value] (label)}
        <dt>{label}</dt>
        <dd>{value}</dd>
      {/each}
    </dl>

    {#if profile.histogram.length > 0}
      {@const largest = Math.max(1, ...profile.histogram)}
      <h3>{L("Distribution")}</h3>
      <div class="histogram" role="img" aria-label={L("Distribution from %@ to %@", profile.min ?? "", profile.max ?? "")}>
        {#each profile.histogram as bin, index (index)}
          <span style:height="{(bin / largest) * 100}%"></span>
        {/each}
      </div>
      <div class="range"><span>{profile.min ?? ""}</span><span>{profile.max ?? ""}</span></div>
    {/if}

    <h3>{L("Problems")}</h3>
    {#if profile.mismatched === 0 && profile.padded === 0}
      <p class="fine">✓ {L("No problems found")}</p>
    {/if}
    {#if profile.mismatched > 0}
      <div class="issue">
        <p>⚠︎ {L("Not %@: %@", kindName(profile.kind).toLowerCase(), count(profile.mismatched))}</p>
        <button class="small" onclick={() => showIssue("mismatch")}>{L("Show")}</button>
      </div>
    {/if}
    {#if profile.padded > 0}
      <div class="issue">
        <p>⚠︎ {L("Spaces around the value: %@", count(profile.padded))}</p>
        <button class="small" onclick={() => showIssue("padding")}>{L("Show")}</button>
        <button class="small" onclick={trim}>{L("Remove")}</button>
      </div>
    {/if}

    {#if profile.topValues.length > 0}
      {@const largest = profile.topValues[0].count || 1}
      <h3>{L("Frequent values")}</h3>
      <ul class="values">
        {#each profile.topValues as item (item.value)}
          <li>
            <button onclick={() => filterValue(item.value)} title={L("Show only rows with this value")}>
              <span class="bar" style:width="{(item.count / largest) * 100}%"></span>
              <span class="value">{item.value === "" ? L("(empty)") : item.value.replace(/\n/g, " ↵ ")}</span>
              <span class="count">{count(item.count)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if loading}
    <p class="loading">…</p>
  {/if}
</aside>

<style>
  .panel {
    width: 280px;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 14px;
    background: var(--background);
    border-left: 1px solid var(--border);
  }

  h2 {
    margin: 0 0 6px;
    font-size: 14px;
    overflow-wrap: anywhere;
  }

  h3 {
    margin: 16px 0 6px;
    font-size: 12px;
  }

  .kind {
    display: inline-block;
    padding: 2px 7px;
    font-size: 11px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-radius: 999px;
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 5px 12px;
    margin: 14px 0 0;
  }

  dt {
    color: var(--muted);
  }

  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
    user-select: text;
    overflow-wrap: anywhere;
  }

  .histogram {
    display: flex;
    align-items: flex-end;
    gap: 1px;
    height: 64px;
  }

  .histogram span {
    flex: 1;
    min-height: 1px;
    background: color-mix(in srgb, var(--accent) 75%, transparent);
    border-radius: 1px 1px 0 0;
  }

  .range {
    display: flex;
    justify-content: space-between;
    margin-top: 3px;
    font-size: 10px;
    color: var(--muted);
  }

  .fine {
    margin: 0;
    color: var(--muted);
  }

  .issue p {
    margin: 0 0 4px;
  }

  .issue {
    margin-bottom: 8px;
  }

  .issue button {
    margin-left: 18px;
  }

  .issue button + button {
    margin-left: 4px;
  }

  .values {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .values button {
    position: relative;
    display: flex;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 3px 6px;
    text-align: left;
    background: none;
    border: 0;
  }

  .bar {
    position: absolute;
    inset: 0 auto 0 0;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-radius: 4px;
  }

  .value {
    position: relative;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .count {
    position: relative;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }

  .loading {
    color: var(--muted);
  }
</style>

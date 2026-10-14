<!-- File ▸ Check Encoding and Delimiter (OpenOptionsView in the macOS app): the start of the
     file as it reads with the chosen encoding and delimiter. Opens by itself when the core is
     not sure how to read a file. -->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { call, type FilePreview, type TextEncoding } from "./lib/api";
  import { describe } from "./lib/errors";
  import { delimiterName, delimiters, encodingName, encodings } from "./lib/formats";
  import { L, count } from "./lib/i18n";

  interface Props {
    path: string;
    encoding: TextEncoding | null;
    delimiter: string | null;
    /** Detection was not sure; the window opened by itself. */
    uncertain: boolean;
    choose: (choice: { encoding: TextEncoding | null; delimiter: string | null } | null) => void;
  }

  let { path, encoding: initialEncoding, delimiter: initialDelimiter, uncertain, choose }: Props = $props();
  const SHOWN_COLUMNS = 8;

  let encoding = $state<TextEncoding | null>(untrack(() => initialEncoding));
  let delimiter = $state<string | null>(untrack(() => initialDelimiter));
  let preview = $state.raw<FilePreview | null>(null);
  let problem = $state("");
  let dialog: HTMLDialogElement;
  let asked = 0;

  async function refresh() {
    const mine = ++asked;
    try {
      const result = await call<FilePreview>("preview", { path, encoding, delimiter, rows: 12 });
      if (mine !== asked) return;
      preview = result;
      problem = "";
    } catch (error) {
      if (mine !== asked) return;
      preview = null;
      problem = describe(error).message;
    }
  }

  const detected = (title: string, value: string | null) => (value ? `${title} (${value})` : title);
  const singleLine = (text: string) => text.replace(/\r\n|\r|\n/g, " ⏎ ");

  onMount(() => {
    dialog.showModal();
    void refresh();
  });
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    choose(null);
  }}
>
  <h2>{uncertain ? L("Check the Encoding and Delimiter") : L("Encoding and Delimiter")}</h2>
  {#if uncertain}
    <p class="note">
      {L("CommaEditor is not sure how to read this file. Choose the encoding and delimiter with which the text reads correctly.")}
    </p>
  {/if}
  <div class="choices">
    <label for="open-encoding">{L("Encoding:")}</label>
    <select id="open-encoding" bind:value={encoding} onchange={refresh}>
      <option value={null}>{detected(L("Detect Automatically"), encoding === null && preview ? encodingName(preview.encoding) : null)}</option>
      <hr />
      {#each encodings as value (value)}<option {value}>{encodingName(value)}</option>{/each}
    </select>
    <label for="open-delimiter">{L("Delimiter:")}</label>
    <select id="open-delimiter" bind:value={delimiter} onchange={refresh}>
      <option value={null}>{detected(L("Detect Automatically"), delimiter === null && preview ? delimiterName(preview.delimiter) : null)}</option>
      <hr />
      {#each delimiters as value (value)}<option {value}>{delimiterName(value)}</option>{/each}
    </select>
  </div>
  <h3>{L("Preview")}</h3>
  <div class="preview">
    {#if preview}
      {@const columns = Math.min(preview.columnCount, SHOWN_COLUMNS)}
      <table>
        <tbody>
          {#each preview.rows as row, index (index)}
            <tr class:header={index === 0 && preview.hasHeader}>
              {#each Array.from({ length: columns }, (_, column) => column) as column (column)}
                <td>{singleLine(row[column] ?? "")}</td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
      {#if preview.columnCount > columns}
        <p class="more">{L("…and %@ more columns", count(preview.columnCount - columns))}</p>
      {/if}
    {:else if problem}
      <p class="problem">{problem}</p>
    {/if}
  </div>
  <footer>
    <button onclick={() => choose(null)}>{uncertain ? L("Keep As Is") : L("Cancel")}</button>
    <button class="default" disabled={!preview} onclick={() => choose({ encoding, delimiter })}>{L("Reopen")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(720px, calc(100vw - 32px));
    padding: 20px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 14px;
  }

  h3 {
    margin: 14px 0 4px;
    font-size: 12px;
  }

  .note {
    margin: 0 0 12px;
    color: var(--muted);
  }

  .choices {
    display: grid;
    grid-template-columns: auto auto;
    justify-content: start;
    gap: 8px;
    align-items: center;
  }

  label {
    text-align: right;
  }

  select {
    font: inherit;
  }

  .preview {
    max-height: 260px;
    overflow: auto;
    padding: 8px;
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  table {
    border-collapse: collapse;
  }

  td {
    max-width: 160px;
    padding: 2px 14px 2px 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .header td {
    font-weight: 600;
  }

  .more {
    margin: 6px 0 0;
    font-size: 11px;
    color: var(--muted);
  }

  .problem {
    margin: 0;
    color: var(--error);
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

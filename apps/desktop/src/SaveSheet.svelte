<!-- The format of a file being saved: encoding, delimiter and line endings, as under the save
     panel of the macOS app (SaveFormatAccessoryView). The file is chosen after it. -->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import type { LineEnding, SaveFormat, TextEncoding } from "./lib/api";
  import { L } from "./lib/i18n";

  interface Props {
    format: SaveFormat;
    choose: (format: SaveFormat | null) => void;
  }

  let { format, choose }: Props = $props();

  const encodings: [TextEncoding, boolean, string][] = [
    ["utf8", false, "UTF-8"],
    ["utf8", true, L("UTF-8 with BOM")],
    ["utf16Le", true, "UTF-16LE"],
    ["utf16Be", true, "UTF-16BE"],
    ["windows1251", false, L("Windows-1251 (Cyrillic)")],
    ["koi8R", false, "KOI8-R"],
    ["ibm866", false, L("CP866 (DOS Cyrillic)")],
    ["koi8U", false, "KOI8-U"],
    ["windows1252", false, L("Windows-1252 (Western)")],
    ["windows1250", false, L("Windows-1250 (Central European)")],
    ["windows1253", false, L("Windows-1253 (Greek)")],
    ["windows1254", false, L("Windows-1254 (Turkish)")],
    ["windows1257", false, L("Windows-1257 (Baltic)")],
  ];
  const lineEndings: [LineEnding, string][] = [
    ["lf", L("LF — macOS, Linux")],
    ["crLf", L("CRLF — Windows")],
    ["cr", L("CR — classic Mac OS")],
  ];
  const initial = untrack(() => format);
  const delimiters: [string, string][] = [
    [",", L("Comma (,)")],
    [";", L("Semicolon (;)")],
    ["\t", L("Tab")],
    ["|", L("Vertical Bar (|)")],
  ];
  if (!delimiters.some(([delimiter]) => delimiter === initial.delimiter)) {
    delimiters.push([initial.delimiter, L("Other: “%@”", initial.delimiter)]);
  }
  const utf16 = (encoding: TextEncoding) => encoding === "utf16Le" || encoding === "utf16Be";

  let encoding = $state(
    Math.max(0, encodings.findIndex(([value, bom]) => value === initial.encoding && (bom === initial.bom || utf16(value)))),
  );
  let delimiter = $state(initial.delimiter);
  let lineEnding = $state<LineEnding>(initial.lineEnding);
  let dialog: HTMLDialogElement;

  /** UTF-8 with BOM, CRLF, and the delimiter Excel expects: `;` where numbers have a decimal comma. */
  function forExcel() {
    encoding = 1;
    lineEnding = "crLf";
    delimiter = new Intl.NumberFormat(navigator.language).format(1.5).includes(",") ? ";" : ",";
  }

  function save() {
    const [value, bom] = encodings[encoding];
    choose({ encoding: value, bom, delimiter, lineEnding });
  }

  onMount(() => dialog.showModal());
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    choose(null);
  }}
>
  <h2>{L("Save As…")}</h2>
  <div class="grid">
    <label for="encoding">{L("Encoding:")}</label>
    <select id="encoding" bind:value={encoding}>
      {#each encodings as [, , label], index (index)}<option value={index}>{label}</option>{/each}
    </select>
    <label for="delimiter">{L("Delimiter:")}</label>
    <select id="delimiter" bind:value={delimiter}>
      {#each delimiters as [value, label] (value)}<option {value}>{label}</option>{/each}
    </select>
    <label for="line-ending">{L("Line Endings:")}</label>
    <select id="line-ending" bind:value={lineEnding}>
      {#each lineEndings as [value, label] (value)}<option {value}>{label}</option>{/each}
    </select>
    <span></span>
    <button class="excel" onclick={forExcel} title={L("UTF-8 with BOM, CRLF and the list separator Excel uses with your region settings")}>
      {L("For Excel")}
    </button>
  </div>
  <footer>
    <button onclick={() => choose(null)}>{L("Cancel")}</button>
    <button class="default" onclick={save}>{L("Save…")}</button>
  </footer>
</dialog>

<style>
  dialog {
    padding: 18px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  h2 {
    margin: 0 0 14px;
    font-size: 13px;
    font-weight: 600;
  }

  .grid {
    display: grid;
    grid-template-columns: auto auto;
    gap: 8px;
    align-items: center;
  }

  label {
    text-align: right;
  }

  select {
    font: inherit;
    min-width: 240px;
  }

  .excel {
    justify-self: start;
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

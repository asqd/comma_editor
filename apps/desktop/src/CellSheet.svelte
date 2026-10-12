<!-- A value with line breaks, edited in a window of its own (CellEditorViewController in the
     macOS app): Cmd-Return on a Mac, Ctrl-Enter elsewhere, saves; Escape cancels. -->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { L } from "./lib/i18n";

  interface Props {
    heading: string;
    value: string;
    save: (value: string) => void;
    cancel: () => void;
  }

  let { heading, value, save, cancel }: Props = $props();
  const mac = navigator.userAgent.includes("Mac");
  // The macOS app names its own keys; elsewhere they are Ctrl and Enter.
  const hint = mac
    ? L("Line breaks are kept. Press ⌘↩ to save.")
    : L("Line breaks are kept. Press ⌘↩ to save.").replace("⌘↩", "Ctrl+Enter");
  let dialog: HTMLDialogElement;
  let text: HTMLTextAreaElement;
  // The draft starts from the value and then is the user's own.
  let draft = $state(untrack(() => value));

  function key(event: KeyboardEvent) {
    if (event.key === "Enter" && (mac ? event.metaKey : event.ctrlKey)) {
      event.preventDefault();
      save(draft);
    }
  }

  onMount(() => {
    dialog.showModal();
    text.focus();
  });
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    cancel();
  }}
>
  <h2>{heading}</h2>
  <textarea bind:this={text} bind:value={draft} onkeydown={key} spellcheck="false" autocomplete="off"></textarea>
  <footer>
    <span class="hint">{hint}</span>
    <button onclick={cancel}>{L("Cancel")}</button>
    <button class="default" onclick={() => save(draft)}>{L("Save")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 32px));
    padding: 16px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  h2 {
    margin: 0 0 10px;
    font-size: 13px;
    font-weight: 600;
  }

  textarea {
    box-sizing: border-box;
    width: 100%;
    height: 240px;
    padding: 6px;
    font: 13px ui-monospace, "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 4px;
    resize: vertical;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }

  .hint {
    flex: 1;
    color: var(--muted);
  }

  .default {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }
</style>

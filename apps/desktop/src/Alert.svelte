<!-- A question or a problem in a window over the table, with buttons and perhaps a text field;
     Escape picks the button marked to cancel, Return the default one. -->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  export interface Button {
    label: string;
    value: string;
    default?: boolean;
    cancel?: boolean;
  }

  interface Props {
    message: string;
    detail?: string;
    buttons: Button[];
    /** A text field starting with this text. */
    field?: string;
    /** Words beside the text field. */
    fieldLabel?: string;
    placeholder?: string;
    /** A checkbox with these words, unchecked at first. */
    checkbox?: string;
    choose: (value: string, text: string, checked: boolean) => void;
  }

  let { message, detail, buttons, field, fieldLabel, placeholder, checkbox, choose }: Props = $props();
  let checked = $state(false);
  let dialog: HTMLDialogElement;
  let input = $state<HTMLInputElement>();
  let text = $state(untrack(() => field ?? ""));

  function submit(event: KeyboardEvent) {
    const fallback = buttons.find((button) => button.default);
    if (event.key === "Enter" && fallback) {
      event.preventDefault();
      choose(fallback.value, text, checked);
    }
  }

  onMount(() => {
    dialog.showModal();
    if (input) input.select();
    else dialog.querySelector<HTMLButtonElement>("button.default")?.focus();
  });
</script>

<dialog
  bind:this={dialog}
  oncancel={(event) => {
    event.preventDefault();
    const cancel = buttons.find((button) => button.cancel);
    if (cancel) choose(cancel.value, text, checked);
  }}
>
  <p class="message">{message}</p>
  {#if detail}<p class="detail">{detail}</p>{/if}
  {#if field !== undefined}
    <div class="field">
      {#if fieldLabel}<label for="alert-field">{fieldLabel}</label>{/if}
      <input id="alert-field" bind:this={input} bind:value={text} onkeydown={submit} {placeholder} spellcheck="false" autocomplete="off" />
    </div>
  {/if}
  {#if checkbox}
    <label class="check"><input type="checkbox" bind:checked />{checkbox}</label>
  {/if}
  <footer>
    {#each buttons as button (button.value)}
      <button class:default={button.default} onclick={() => choose(button.value, text, checked)}>{button.label}</button>
    {/each}
  </footer>
</dialog>

<style>
  dialog {
    width: min(420px, calc(100vw - 32px));
    padding: 18px;
    color: var(--text);
    background: var(--background);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.25);
  }

  .message {
    margin: 0 0 8px;
    font-weight: 600;
  }

  .detail {
    margin: 0 0 8px;
    color: var(--muted);
  }

  .field {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 10px;
  }

  .field input {
    box-sizing: border-box;
    flex: 1;
    margin-top: 4px;
    padding: 4px 6px;
    font: inherit;
    color: var(--text);
    background: var(--grid-background);
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  footer {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 14px;
  }

  .default {
    color: white;
    background: var(--accent);
    border-color: var(--accent);
  }
</style>

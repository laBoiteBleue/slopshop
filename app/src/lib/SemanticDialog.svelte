<script lang="ts">
  // Select > Semantic…: a text naming what to select ("sky", "the red car"); SAM 3 selects
  // every instance of it. It understands English noun phrases.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";

  let {
    value,
    onapply,
    onclose,
  }: {
    /** The text used last. */
    value: string;
    onapply: (text: string) => void;
    onclose: () => void;
  } = $props();

  let text = $state("");
  let dialog: HTMLDialogElement;
  let input: HTMLInputElement;

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (text.trim()) onapply(text.trim());
  }

  onMount(() => {
    text = value;
    dialog.showModal();
    input.select();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="semantic-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="semantic-title">{t("semantic.title")}</header>
    <div class="body">
      <label for="semantic-text">{t("semantic.label")}</label>
      <input
        id="semantic-text"
        type="text"
        maxlength="200"
        spellcheck="false"
        autocomplete="off"
        bind:this={input}
        bind:value={text}
      />
      <p class="hint">{t("semantic.hint")}</p>
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!text.trim()}>
        {t("sizeDialog.ok")}
      </button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 340px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    gap: 6px;
    padding: 10px;
  }

  label,
  .hint {
    color: var(--text-muted);
  }

  .hint {
    margin: 0;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>

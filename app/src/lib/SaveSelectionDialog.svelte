<script lang="ts">
  // Select > Save Selection: a name for the selection, kept in the document. A name already
  // used says so, and OK then replaces that saved selection (as Photoshop's channel would be).
  import { onMount, untrack } from "svelte";
  import type { SavedSelectionView } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { nextSelectionName, savedNamed } from "./savedSelections";

  let {
    saved,
    onapply,
    onclose,
  }: {
    /** The document's saved selections. */
    saved: SavedSelectionView[];
    /** The name, and the saved selection it replaces (`null`: a new one). */
    onapply: (name: string, replace: number | null) => void;
    onclose: () => void;
  } = $props();

  let name = $state(
    untrack(() => nextSelectionName(saved, (n) => t("saveSelection.default", { n }))),
  );
  let dialog: HTMLDialogElement;
  let field: HTMLInputElement;
  const replaced = $derived(savedNamed(saved, name));
  const valid = $derived(name.trim().length > 0);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(name.trim(), replaced?.id ?? null);
  }

  onMount(() => {
    dialog.showModal();
    field.select();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="save-selection-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="save-selection-title">{t("saveSelection.title")}</header>
    <div class="fields">
      <label for="save-selection-name">{t("saveSelection.name")}</label>
      <input
        id="save-selection-name"
        type="text"
        maxlength="200"
        spellcheck="false"
        bind:this={field}
        bind:value={name}
      />
      {#if replaced}
        <p class="note" role="status">{t("saveSelection.replaces", { name: replaced.name })}</p>
      {/if}
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>
        {t(replaced ? "saveSelection.replace" : "sizeDialog.ok")}
      </button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 300px;
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

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 6px 10px;
    padding: 12px 10px;
  }

  .fields > label {
    color: var(--text-muted);
  }

  input {
    min-width: 0;
  }

  .note {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--text-muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>

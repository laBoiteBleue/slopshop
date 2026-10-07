<script lang="ts">
  // Edit > Define Pattern (ADR 0042): a name for the pattern made of what the image shows
  // within the selection (the whole canvas without one), added to the library.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    name: initial,
    onapply,
    onclose,
  }: {
    /** The name proposed. */
    name: string;
    onapply: (name: string) => void;
    onclose: () => void;
  } = $props();

  // The proposal only seeds the field.
  let name = $state(untrack(() => initial));
  let dialog: HTMLDialogElement;
  let field: HTMLInputElement;
  const valid = $derived(name.trim().length > 0);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(name.trim());
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
  aria-labelledby="define-pattern-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="define-pattern-title" {@attach movable("define-pattern")}>
      {t("patterns.define.title")}
    </header>
    <div class="fields">
      <label for="define-pattern-name">{t("patterns.define.name")}</label>
      <input
        id="define-pattern-name"
        type="text"
        maxlength="200"
        spellcheck="false"
        bind:this={field}
        bind:value={name}
      />
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
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

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>

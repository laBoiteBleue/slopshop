<script lang="ts">
  // Layer > Lock Layers, Photoshop's dialog (ADR 0046): the locks the selected layers get, all
  // of them at once. It starts from the locks they all have.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import type { MessageKey } from "./i18n/en";
  import type { LockKind, Locks } from "./locks";

  let {
    locks,
    onapply,
    onclose,
  }: {
    /** What the dialog starts from. */
    locks: Locks;
    onapply: (locks: Locks) => void;
    onclose: () => void;
  } = $props();

  // In the order of Photoshop's dialog.
  const KINDS: [LockKind, MessageKey][] = [
    ["all", "lockDialog.all"],
    ["transparency", "lockDialog.transparency"],
    ["pixels", "lockDialog.pixels"],
    ["position", "lockDialog.position"],
  ];

  let chosen = $state(untrack(() => ({ ...locks })));
  let dialog: HTMLDialogElement;

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onapply(chosen);
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="lock-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="lock-title" {@attach movable("lockLayers")}>{t("lockDialog.title")}</header>
    <fieldset>
      <legend>{t("lockDialog.lock")}</legend>
      {#each KINDS as [kind, label] (kind)}
        <label>
          <input type="checkbox" bind:checked={chosen[kind]} />
          {t(label)}
        </label>
      {/each}
    </fieldset>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary">{t("sizeDialog.ok")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 240px;
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

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 10px;
    padding: 6px 10px 8px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
  }

  legend {
    padding: 0 4px;
    color: var(--text-muted);
  }

  label {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>

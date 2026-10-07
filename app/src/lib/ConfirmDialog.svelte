<script lang="ts">
  // A question before something hard to take back at a glance (deleting a source and the
  // layers showing it): what will happen, the items it touches, and the action or Cancel. The
  // action stays undoable; the dialog says what it does before it does it.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    title,
    message,
    items = [],
    action,
    onconfirm,
    onclose,
  }: {
    title: string;
    message: string;
    /** What it touches (layer names…), listed under the message. */
    items?: string[];
    /** The confirming button's label. */
    action: string;
    onconfirm: () => void;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;

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
  aria-labelledby="confirm-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onconfirm();
    }}
  >
    <header id="confirm-title" {@attach movable("confirm")}>{title}</header>
    <div class="body">
      <p>{message}</p>
      {#if items.length > 0}
        <ul>
          {#each items as item, i (i)}
            <li>{item}</li>
          {/each}
        </ul>
      {/if}
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary">{action}</button>
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

  .body {
    padding: 12px 10px;
  }

  p {
    margin: 0 0 6px;
  }

  ul {
    max-height: 160px;
    margin: 0;
    padding-left: 18px;
    overflow-y: auto;
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

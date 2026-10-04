<script lang="ts">
  // Image > Trim, Photoshop's dialog: what a margin is (transparent pixels, or the color of a
  // corner pixel) and the sides it is taken off. Nothing is deleted: it is a crop.
  import { onMount, untrack } from "svelte";
  import type { TrimBasis, TrimSettings } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import type { MessageKey } from "./i18n/en";

  let {
    settings,
    onapply,
    onclose,
  }: {
    /** The settings used last. */
    settings: TrimSettings;
    onapply: (settings: TrimSettings) => void;
    onclose: () => void;
  } = $props();

  const BASES: [TrimBasis, MessageKey][] = [
    ["transparent", "trimDialog.transparent"],
    ["topLeft", "trimDialog.topLeft"],
    ["bottomRight", "trimDialog.bottomRight"],
  ];
  const SIDES: ["top" | "bottom" | "left" | "right", MessageKey][] = [
    ["top", "trimDialog.top"],
    ["left", "trimDialog.left"],
    ["bottom", "trimDialog.bottom"],
    ["right", "trimDialog.right"],
  ];

  let chosen = $state(untrack(() => ({ ...settings })));
  let dialog: HTMLDialogElement;
  const valid = $derived(chosen.top || chosen.bottom || chosen.left || chosen.right);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(chosen);
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
  aria-labelledby="trim-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="trim-title" {@attach movable("trim")}>{t("trimDialog.title")}</header>
    <fieldset>
      <legend>{t("trimDialog.basedOn")}</legend>
      {#each BASES as [basis, label] (basis)}
        <label>
          <input type="radio" bind:group={chosen.basis} value={basis} />
          {t(label)}
        </label>
      {/each}
    </fieldset>
    <fieldset class="sides">
      <legend>{t("trimDialog.trimAway")}</legend>
      {#each SIDES as [side, label] (side)}
        <label>
          <input type="checkbox" bind:checked={chosen[side]} />
          {t(label)}
        </label>
      {/each}
    </fieldset>
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

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 10px;
    padding: 6px 10px 8px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
  }

  .sides {
    display: grid;
    grid-template-columns: 1fr 1fr;
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

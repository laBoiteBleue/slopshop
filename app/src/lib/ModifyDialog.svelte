<script lang="ts">
  // Select > Modify, as in Photoshop: one number of pixels (Border's width, Smooth's sample
  // radius, Expand's and Contract's amounts, Feather's radius), applied to the whole selection.
  // Also Select > Refine Edge's radius (the band ViTMatte decides around the outline).
  import { onMount, untrack } from "svelte";
  import type { SelectionModify } from "./engine";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";

  let {
    kind,
    value,
    max,
    onapply,
    onclose,
  }: {
    kind: SelectionModify | "refine";
    /** The value used last for this kind. */
    value: number;
    /** The largest value accepted, in pixels. */
    max: number;
    onapply: (value: number) => void;
    onclose: () => void;
  } = $props();

  const TITLES: Record<SelectionModify | "refine", MessageKey> = {
    refine: "modify.refine.title",
    border: "modify.border.title",
    smooth: "modify.smooth.title",
    expand: "modify.expand.title",
    contract: "modify.contract.title",
    feather: "modify.feather.title",
  };
  const LABELS: Record<SelectionModify | "refine", MessageKey> = {
    refine: "modify.refine.label",
    border: "modify.border.label",
    smooth: "modify.smooth.label",
    expand: "modify.expand.label",
    contract: "modify.contract.label",
    feather: "modify.feather.label",
  };

  let amount = $state(untrack(() => value));
  let dialog: HTMLDialogElement;
  const valid = $derived(Number.isFinite(amount) && amount > 0 && amount <= max);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(amount);
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
  aria-labelledby="modify-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="modify-title">{t(TITLES[kind])}</header>
    <div class="fields">
      <label for="modify-amount">{t(LABELS[kind])}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="modify-amount"
        type="number"
        min="0"
        {max}
        step="any"
        autofocus
        bind:value={amount}
      />
      <span>{t("modify.pixels")}</span>
      <!-- Logarithmic, as a brush size: small amounts as easy to set as large ones. -->
      <input
        class="slider"
        type="range"
        min="0"
        max="1000"
        step="1"
        aria-label={t(LABELS[kind])}
        value={(Math.log(Math.max(amount, 1)) / Math.log(max)) * 1000}
        oninput={(e) => (amount = Math.round(Math.pow(max, e.currentTarget.valueAsNumber / 1000)))}
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
    width: 280px;
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
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 6px 10px;
    padding: 12px 10px;
  }

  .fields > label {
    color: var(--text-muted);
  }

  input[type="number"] {
    min-width: 0;
  }

  .slider {
    grid-column: 1 / -1;
    accent-color: var(--accent);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>

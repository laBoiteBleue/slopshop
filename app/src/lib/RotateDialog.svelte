<script lang="ts">
  // Image > Image Rotation > Arbitrary, Photoshop's Rotate Canvas dialog: an angle and its
  // direction. The canvas grows to hold the turned image (the engine's `rotateImageBy`).
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  /** Photoshop's limit: less than a whole turn either way. */
  const MAX_ANGLE = 359.99;

  let {
    angle,
    clockwise,
    onapply,
    onclose,
  }: {
    /** The angle used last, in degrees (positive). */
    angle: number;
    /** The direction used last. */
    clockwise: boolean;
    onapply: (angle: number, clockwise: boolean) => void;
    onclose: () => void;
  } = $props();

  let amount = $state(untrack(() => angle));
  let cw = $state(untrack(() => clockwise));
  let dialog: HTMLDialogElement;
  const valid = $derived(Number.isFinite(amount) && Math.abs(amount) <= MAX_ANGLE);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(amount, cw);
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
  aria-labelledby="rotate-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="rotate-title" {@attach movable("rotate")}>{t("rotateDialog.title")}</header>
    <div class="fields">
      <label for="rotate-angle">{t("rotateDialog.angle")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="rotate-angle"
        type="number"
        min={-MAX_ANGLE}
        max={MAX_ANGLE}
        step="any"
        autofocus
        bind:value={amount}
      />
      <div class="directions" role="radiogroup">
        <label>
          <input type="radio" bind:group={cw} value={true} />
          {t("rotateDialog.cw")}
        </label>
        <label>
          <input type="radio" bind:group={cw} value={false} />
          {t("rotateDialog.ccw")}
        </label>
      </div>
      <input
        class="slider"
        type="range"
        min="-180"
        max="180"
        step="0.1"
        aria-label={t("rotateDialog.angle")}
        value={cw ? amount : -amount}
        oninput={(e) => {
          const v = e.currentTarget.valueAsNumber;
          amount = Math.abs(v);
          cw = v >= 0;
        }}
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

  .directions {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .directions label {
    display: flex;
    align-items: center;
    gap: 4px;
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

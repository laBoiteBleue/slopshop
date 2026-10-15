<script lang="ts">
  // Layer > Layer Style > Scale Effects, Photoshop's Scale Layer Effects dialog: one percentage
  // scaling every size and distance of the selected layers' styles. The canvas follows each
  // change (the app sends it live); OK keeps it as one undo entry, Cancel takes it back.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import { EFFECTS_SCALE } from "./layerStyle";

  let {
    onchange,
    onok,
    oncancel,
  }: {
    /** The scale changed, percent: the canvas follows it. */
    onchange: (scale: number) => void;
    onok: () => void;
    oncancel: () => void;
  } = $props();

  let scale = $state(100);
  let dialog: HTMLDialogElement;
  const valid = $derived(
    Number.isFinite(scale) && scale >= EFFECTS_SCALE.min && scale <= EFFECTS_SCALE.max,
  );

  function set(value: number) {
    scale = value;
    if (valid) onchange(scale);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onok();
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
  aria-labelledby="scale-effects-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <form onsubmit={submit}>
    <header id="scale-effects-title" {@attach movable("scaleEffects")}>
      {t("scaleEffects.title")}
    </header>
    <div class="fields">
      <label for="scale-effects">{t("scaleEffects.scale")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="scale-effects"
        type="number"
        min={EFFECTS_SCALE.min}
        max={EFFECTS_SCALE.max}
        step="1"
        autofocus
        value={scale}
        oninput={(e) => set(e.currentTarget.valueAsNumber)}
      />
      <span>%</span>
      <input
        class="slider"
        type="range"
        min={EFFECTS_SCALE.min}
        max={EFFECTS_SCALE.max}
        step="1"
        aria-label={t("scaleEffects.scale")}
        value={scale}
        oninput={(e) => set(e.currentTarget.valueAsNumber)}
      />
    </div>
    <footer>
      <button type="button" class="btn" onclick={oncancel}>{t("sizeDialog.cancel")}</button>
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

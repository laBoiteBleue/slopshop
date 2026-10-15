<script lang="ts">
  // Layer > Layer Style > Global Light, Photoshop's Global Light dialog: the document's light,
  // its angle and altitude, shared by Drop Shadow, Inner Shadow and Bevel and Emboss where they
  // use it. The canvas follows each change (the app sends it live); OK keeps it as one undo
  // entry, Cancel takes it back.
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import type { GlobalLight } from "./layerStyle";

  let {
    light,
    onchange,
    onok,
    oncancel,
  }: {
    /** The document's light as the dialog opens. */
    light: GlobalLight;
    /** The light changed: the canvas follows it. */
    onchange: (light: GlobalLight) => void;
    onok: () => void;
    oncancel: () => void;
  } = $props();

  let angle = $state(untrack(() => light[0]));
  let altitude = $state(untrack(() => light[1]));
  let dialog: HTMLDialogElement;
  const valid = $derived(
    Number.isFinite(angle) &&
      Math.abs(angle) <= 180 &&
      Number.isFinite(altitude) &&
      altitude >= 0 &&
      altitude <= 90,
  );

  function set(next: { angle?: number; altitude?: number }) {
    angle = next.angle ?? angle;
    altitude = next.altitude ?? altitude;
    if (valid) onchange([angle, altitude]);
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
  aria-labelledby="global-light-title"
  oncancel={(e) => {
    e.preventDefault();
    oncancel();
  }}
>
  <form onsubmit={submit}>
    <header id="global-light-title" {@attach movable("globalLight")}>
      {t("globalLight.title")}
    </header>
    <div class="fields">
      <label for="global-light-angle">{t("globalLight.angle")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="global-light-angle"
        type="number"
        min="-180"
        max="180"
        step="1"
        autofocus
        value={angle}
        oninput={(e) => set({ angle: e.currentTarget.valueAsNumber })}
      />
      <span>°</span>
      <input
        class="slider"
        type="range"
        min="-180"
        max="180"
        step="1"
        aria-label={t("globalLight.angle")}
        value={angle}
        oninput={(e) => set({ angle: e.currentTarget.valueAsNumber })}
      />
      <label for="global-light-altitude">{t("globalLight.altitude")}</label>
      <input
        id="global-light-altitude"
        type="number"
        min="0"
        max="90"
        step="1"
        value={altitude}
        oninput={(e) => set({ altitude: e.currentTarget.valueAsNumber })}
      />
      <span>°</span>
      <input
        class="slider"
        type="range"
        min="0"
        max="90"
        step="1"
        aria-label={t("globalLight.altitude")}
        value={altitude}
        oninput={(e) => set({ altitude: e.currentTarget.valueAsNumber })}
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

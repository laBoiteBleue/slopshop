<script lang="ts">
  // Image > Image Size and Image > Canvas Size, as in Photoshop: a width and a height in pixels or
  // percent. Image Size resamples the whole image (proportions linked by default); Canvas Size
  // changes the canvas around the image, kept where the anchor says (relative sizes add to the
  // current ones). Nothing is cut or rewritten: layers are transformed (ADR 0017, 0018).
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";

  let {
    mode,
    width,
    height,
    onapply,
    onclose,
  }: {
    mode: "image" | "canvas";
    /** The current size, in pixels. */
    width: number;
    height: number;
    /** Apply the new size (and, for the canvas, the anchor: [x, y] in [0, 1]). */
    onapply: (width: number, height: number, anchor: [number, number]) => void;
    onclose: () => void;
  } = $props();

  /** Largest side accepted, in pixels. */
  const MAX_SIDE = 300_000;

  const current = untrack(() => ({ width, height }));
  let unit = $state<"px" | "percent">("px");
  let relative = $state(false);
  let constrain = $state(true);
  let anchor = $state<[number, number]>([0.5, 0.5]);
  /** The fields, in `unit`. */
  let w = $state(current.width);
  let h = $state(current.height);
  let dialog: HTMLDialogElement;

  /** A field's value as pixels, for a side currently `size` pixels long. */
  function pixels(value: number, size: number): number {
    const base = relative ? size : 0;
    return Math.round(unit === "px" ? base + value : (size * value) / 100 + (relative ? size : 0));
  }

  const newWidth = $derived(pixels(w, current.width));
  const newHeight = $derived(pixels(h, current.height));
  const valid = $derived(
    Number.isFinite(newWidth) &&
      Number.isFinite(newHeight) &&
      newWidth >= 1 &&
      newHeight >= 1 &&
      newWidth <= MAX_SIDE &&
      newHeight <= MAX_SIDE,
  );

  /** The fields' values for `unit` and `relative`, from pixel sizes. */
  function fields(width: number, height: number) {
    const value = (px: number, size: number) => {
      const extra = relative ? px - size : px;
      return unit === "px" ? extra : Math.round((extra / size) * 10000) / 100;
    };
    w = value(width, current.width);
    h = value(height, current.height);
  }

  function setUnit(next: "px" | "percent") {
    const [width, height] = [newWidth, newHeight];
    unit = next;
    fields(width, height);
  }

  function setRelative(next: boolean) {
    const [width, height] = [newWidth, newHeight];
    relative = next;
    fields(width, height);
  }

  // Image Size keeps the proportions when asked: one field follows the other.
  function onWidth() {
    if (mode !== "image" || !constrain) return;
    h = unit === "percent" ? w : Math.round((w * current.height) / current.width);
  }

  function onHeight() {
    if (mode !== "image" || !constrain) return;
    w = unit === "percent" ? h : Math.round((h * current.width) / current.height);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(newWidth, newHeight, $state.snapshot(anchor));
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });

  const ANCHORS = [0, 0.5, 1];
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="size-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="size-title">
      {t(mode === "image" ? "sizeDialog.imageTitle" : "sizeDialog.canvasTitle")}
    </header>
    <p class="current">
      {t("sizeDialog.current", { width: current.width, height: current.height })}
    </p>
    <div class="fields">
      <label for="size-width">{t("sizeDialog.width")}</label>
      <input id="size-width" type="number" step="any" bind:value={w} oninput={onWidth} />
      <label for="size-height">{t("sizeDialog.height")}</label>
      <input id="size-height" type="number" step="any" bind:value={h} oninput={onHeight} />
      <label for="size-unit">{t("sizeDialog.unit")}</label>
      <select
        id="size-unit"
        value={unit}
        onchange={(e) => setUnit((e.currentTarget as HTMLSelectElement).value as "px" | "percent")}
      >
        <option value="px">{t("sizeDialog.unit.px")}</option>
        <option value="percent">{t("sizeDialog.unit.percent")}</option>
      </select>
      {#if mode === "image"}
        <label class="check">
          <input type="checkbox" bind:checked={constrain} onchange={onWidth} />
          {t("sizeDialog.constrain")}
        </label>
      {:else}
        <label class="check">
          <input
            type="checkbox"
            checked={relative}
            onchange={(e) => setRelative((e.currentTarget as HTMLInputElement).checked)}
          />
          {t("sizeDialog.relative")}
        </label>
        <span class="label">{t("sizeDialog.anchor")}</span>
        <div class="anchor" role="radiogroup" aria-label={t("sizeDialog.anchor")}>
          {#each ANCHORS as y (y)}
            {#each ANCHORS as x (x)}
              <button
                type="button"
                role="radio"
                aria-checked={anchor[0] === x && anchor[1] === y}
                aria-label={t("sizeDialog.anchor")}
                class:selected={anchor[0] === x && anchor[1] === y}
                onclick={() => (anchor = [x, y])}
              ></button>
            {/each}
          {/each}
        </div>
      {/if}
      <p class="result" class:invalid={!valid}>
        {t("sizeDialog.new", { width: newWidth, height: newHeight })}
      </p>
    </div>
    <footer>
      <button type="button" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
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

  .current {
    margin: 0;
    padding: 6px 10px 0;
    color: var(--text-muted);
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 6px 10px;
    padding: 10px;
  }

  .fields > label:not(.check),
  .label {
    color: var(--text-muted);
  }

  input[type="number"],
  select {
    min-width: 0;
  }

  .check {
    grid-column: 2;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .check input {
    margin: 0;
    accent-color: var(--accent);
  }

  .anchor {
    display: grid;
    grid-template-columns: repeat(3, 18px);
    gap: 2px;
  }

  .anchor button {
    width: 18px;
    height: 18px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 2px;
    background: transparent;
  }

  .anchor button.selected {
    background: var(--accent);
    border-color: var(--accent);
  }

  .result {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--text-muted);
  }

  .result.invalid {
    color: var(--danger-fg);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }

  footer button {
    min-width: 76px;
  }
</style>

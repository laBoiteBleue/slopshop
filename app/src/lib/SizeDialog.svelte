<script lang="ts">
  // Image > Image Size and Image > Canvas Size, as in Photoshop. Sizes in percent, pixels,
  // inches, centimeters or millimeters, at the document's resolution (ADR 0028).
  // - Image Size resamples the whole image (proportions linked by default) and sets the
  //   resolution (pixels per inch or per centimeter). With Resample off, the pixels stay: a size
  //   in inches or centimeters changes the resolution instead, and the proportions are kept.
  // - Canvas Size changes the canvas around the image, kept where the anchor says (relative
  //   sizes add to the current ones).
  // Nothing is cut or rewritten: layers are transformed (ADR 0017, 0018).
  import { onMount, untrack } from "svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import Icon from "./Icon.svelte";
  import {
    LENGTH_UNITS,
    MAX_PPI,
    MIN_PPI,
    RESOLUTION_UNITS,
    fromPixels,
    fromPpi,
    rounded,
    toPixels,
    toPpi,
    type LengthUnit,
    type ResolutionUnit,
  } from "./units";

  let {
    mode,
    width,
    height,
    resolution,
    onapply,
    onclose,
  }: {
    mode: "image" | "canvas";
    /** The current size, in pixels. */
    width: number;
    height: number;
    /** The document's resolution, pixels per inch. */
    resolution: number;
    /**
     * Apply the new size, the anchor (Canvas Size: [x, y] in [0, 1]) and the resolution (Image
     * Size, pixels per inch).
     */
    onapply: (width: number, height: number, anchor: [number, number], resolution: number) => void;
    onclose: () => void;
  } = $props();

  /** Largest side accepted, in pixels. */
  const MAX_SIDE = 300_000;

  type Unit = LengthUnit | "percent";
  const UNITS: Unit[] = ["percent", ...LENGTH_UNITS];
  const UNIT_LABELS: Record<Unit, MessageKey> = {
    percent: "sizeDialog.unit.percent",
    px: "sizeDialog.unit.px",
    in: "units.in",
    cm: "units.cm",
    mm: "units.mm",
  };
  const RESOLUTION_LABELS: Record<ResolutionUnit, MessageKey> = {
    ppi: "units.ppi",
    ppcm: "units.ppcm",
  };

  const current = untrack(() => ({ width, height, ppi: resolution }));
  let unit = $state<Unit>("px");
  let resolutionUnit = $state<ResolutionUnit>("ppi");
  let relative = $state(false);
  let constrain = $state(true);
  let resample = $state(true);
  let anchor = $state<[number, number]>([0.5, 0.5]);
  /** The new size in pixels (not rounded while typing) and resolution, pixels per inch. */
  let pxWidth = $state(current.width);
  let pxHeight = $state(current.height);
  let ppi = $state(current.ppi);
  /** The fields as typed: rewritten only when something else changes them. */
  let wField = $state("");
  let hField = $state("");
  let rField = $state("");
  let dialog: HTMLDialogElement;

  const newWidth = $derived(Math.round(pxWidth));
  const newHeight = $derived(Math.round(pxHeight));
  const valid = $derived(
    [newWidth, newHeight].every((side) => Number.isFinite(side) && side >= 1 && side <= MAX_SIDE) &&
      Number.isFinite(ppi) &&
      ppi >= MIN_PPI &&
      ppi <= MAX_PPI,
  );
  /** Without resampling, the pixels are fixed: they cannot be typed in pixels or percent. */
  const locked = $derived(mode === "image" && !resample && (unit === "px" || unit === "percent"));

  /** A side of `pixels` (`size` now) in the fields' unit. */
  function shown(pixels: number, size: number): number {
    const value = relative ? pixels - size : pixels;
    return unit === "percent"
      ? rounded((value / size) * 100, "percent")
      : rounded(fromPixels(value, unit, ppi), unit);
  }

  /** A field's value as pixels, for a side `size` pixels long now. */
  function typed(value: number, size: number): number {
    const extra = unit === "percent" ? (size * value) / 100 : toPixels(value, unit, ppi);
    return relative ? size + extra : extra;
  }

  /** Every field from the state. */
  function sync() {
    wField = String(shown(pxWidth, current.width));
    hField = String(shown(pxHeight, current.height));
    rField = String(rounded(fromPpi(ppi, resolutionUnit), resolutionUnit));
  }
  sync();

  function setUnit(next: Unit) {
    unit = next;
    sync();
  }

  function setRelative(next: boolean) {
    relative = next;
    sync();
  }

  /** A width or a height typed. */
  function onSide(side: "width" | "height") {
    const value = Number(side === "width" ? wField : hField);
    if (!Number.isFinite(value)) return;
    const size = side === "width" ? current.width : current.height;
    const pixels = typed(value, size);
    if (mode === "image" && !resample) {
      // The pixels stay: the length typed is the print size, which sets the resolution.
      if (pixels <= 0) return;
      ppi = (ppi * size) / pixels;
      rField = String(rounded(fromPpi(ppi, resolutionUnit), resolutionUnit));
      if (side === "width") hField = String(shown(pxHeight, current.height));
      else wField = String(shown(pxWidth, current.width));
      return;
    }
    if (side === "width") pxWidth = pixels;
    else pxHeight = pixels;
    // Image Size keeps the proportions when asked: the other side follows.
    if (mode === "image" && constrain) {
      if (side === "width") {
        pxHeight = (pxWidth * current.height) / current.width;
        hField = String(shown(pxHeight, current.height));
      } else {
        pxWidth = (pxHeight * current.width) / current.height;
        wField = String(shown(pxWidth, current.width));
      }
    }
  }

  /** The resolution typed: with resampling the print size stays (the pixels follow). */
  function onResolution() {
    const next = toPpi(Number(rField), resolutionUnit);
    if (!Number.isFinite(next) || next <= 0) return;
    if (resample) {
      pxWidth = (pxWidth * next) / ppi;
      pxHeight = (pxHeight * next) / ppi;
    }
    ppi = next;
    // Pixel fields change with resampling, length fields without.
    if ((unit === "px") === resample || unit === "percent") {
      wField = String(shown(pxWidth, current.width));
      hField = String(shown(pxHeight, current.height));
    }
  }

  function setResolutionUnit(next: ResolutionUnit) {
    resolutionUnit = next;
    rField = String(rounded(fromPpi(ppi, resolutionUnit), resolutionUnit));
  }

  function setResample(next: boolean) {
    resample = next;
    if (!resample) {
      // Back to the image's pixels; their print size at the resolution.
      pxWidth = current.width;
      pxHeight = current.height;
      if (unit === "px" || unit === "percent") unit = "cm";
      sync();
    }
  }

  function toggleConstrain() {
    constrain = !constrain;
    if (constrain) onSide("width");
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(newWidth, newHeight, $state.snapshot(anchor), ppi);
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
      <input
        id="size-width"
        type="number"
        step="any"
        disabled={locked}
        bind:value={wField}
        oninput={() => onSide("width")}
      />
      {#if mode === "image"}
        <!-- Photoshop's link between the width and the height: a bracket and a chain while the
             proportions are kept, the chain open otherwise; a click toggles them. Without
             resampling the proportions are always kept. -->
        <button
          type="button"
          class="link"
          class:on={constrain || !resample}
          aria-pressed={constrain || !resample}
          disabled={!resample}
          title={t("sizeDialog.constrain")}
          aria-label={t("sizeDialog.constrain")}
          onclick={toggleConstrain}
        >
          <Icon name={constrain || !resample ? "link" : "linkBroken"} size={14} />
        </button>
      {/if}
      <label for="size-height">{t("sizeDialog.height")}</label>
      <input
        id="size-height"
        type="number"
        step="any"
        disabled={locked}
        bind:value={hField}
        oninput={() => onSide("height")}
      />
      <label for="size-unit">{t("sizeDialog.unit")}</label>
      <select
        id="size-unit"
        value={unit}
        onchange={(e) => setUnit((e.currentTarget as HTMLSelectElement).value as Unit)}
      >
        {#each UNITS as option (option)}
          <option value={option}>{t(UNIT_LABELS[option])}</option>
        {/each}
      </select>
      {#if mode === "image"}
        <label for="size-resolution">{t("sizeDialog.resolution")}</label>
        <div class="with-unit">
          <input
            id="size-resolution"
            type="number"
            step="any"
            min="0"
            bind:value={rField}
            oninput={onResolution}
          />
          <select
            aria-label={t("sizeDialog.resolution")}
            value={resolutionUnit}
            onchange={(e) =>
              setResolutionUnit((e.currentTarget as HTMLSelectElement).value as ResolutionUnit)}
          >
            {#each RESOLUTION_UNITS as option (option)}
              <option value={option}>{t(RESOLUTION_LABELS[option])}</option>
            {/each}
          </select>
        </div>
        <label class="check">
          <input
            type="checkbox"
            checked={resample}
            onchange={(e) => setResample((e.currentTarget as HTMLInputElement).checked)}
          />
          {t("sizeDialog.resample")}
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
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>{t("sizeDialog.ok")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 320px;
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
    /* The third column holds the link between the width and the height (Image Size). */
    grid-template-columns: auto 1fr 16px;
    align-items: center;
    gap: 6px 10px;
    padding: 10px;
  }

  /* Each label starts a row, whether or not the link takes the third column. */
  .fields > label:not(.check),
  .label {
    grid-column: 1;
    color: var(--text-muted);
  }

  .link {
    position: relative;
    grid-column: 3;
    grid-row: span 2;
    align-self: stretch;
    display: grid;
    place-items: center;
    width: 16px;
    margin-left: -6px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
  }

  .link:hover {
    color: var(--text);
  }

  /* The bracket from the middle of the width field to the middle of the height field. */
  .link.on::before {
    content: "";
    position: absolute;
    top: 25%;
    bottom: 25%;
    left: 0;
    width: 8px;
    border: 1px solid var(--text-muted);
    border-left: 0;
  }

  .link :global(svg) {
    position: relative;
    margin-left: 0;
    background: var(--panel);
  }

  .link.on {
    color: var(--text);
  }

  input[type="number"],
  select {
    min-width: 0;
  }

  .with-unit {
    display: flex;
    gap: 6px;
    min-width: 0;
  }

  .with-unit input {
    flex: 1;
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
</style>

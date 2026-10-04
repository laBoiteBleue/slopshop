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
  import { movable } from "./dialogDrag";
  import type { MessageKey } from "./i18n/en";
  import Icon from "./Icon.svelte";
  import {
    LENGTH_UNITS,
    RESOLUTION_UNITS,
    fromPpi,
    rounded,
    toPpi,
    type ResolutionUnit,
  } from "./units";
  import {
    initialSize,
    isLocked,
    isValid,
    newSize,
    sideField,
    typeResolution,
    typeSide,
    withResample,
    type SizeField,
    type SizeState,
    type SizeUnit,
  } from "./sizeDialog";

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

  const UNITS: SizeUnit[] = ["percent", ...LENGTH_UNITS];
  const UNIT_LABELS: Record<SizeUnit, MessageKey> = {
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
  let size = $state<SizeState>(untrack(() => initialSize(mode, current)));
  let resolutionUnit = $state<ResolutionUnit>("ppi");
  let anchor = $state<[number, number]>([0.5, 0.5]);
  /** The fields as typed: rewritten only when something else changes them. */
  let wField = $state("");
  let hField = $state("");
  let rField = $state("");
  let dialog: HTMLDialogElement;

  const applied = $derived(newSize(size));
  const valid = $derived(isValid(size));
  const locked = $derived(isLocked(size));

  /** Rewrite `fields` from the state. */
  function rewrite(fields: SizeField[]) {
    for (const field of fields) {
      if (field === "width") wField = String(sideField(size, "width"));
      else if (field === "height") hField = String(sideField(size, "height"));
      else rField = String(rounded(fromPpi(size.ppi, resolutionUnit), resolutionUnit));
    }
  }

  /** Every field from the state. */
  function sync() {
    rewrite(["width", "height", "resolution"]);
  }
  sync();

  function setUnit(next: SizeUnit) {
    size.unit = next;
    sync();
  }

  function setRelative(next: boolean) {
    size.relative = next;
    sync();
  }

  function apply(change: { state: SizeState; rewrite: SizeField[] } | null) {
    if (!change) return;
    size = change.state;
    rewrite(change.rewrite);
  }

  /** A width or a height typed. */
  function onSide(side: "width" | "height") {
    apply(typeSide(size, side, Number(side === "width" ? wField : hField)));
  }

  function onResolution() {
    apply(typeResolution(size, toPpi(Number(rField), resolutionUnit)));
  }

  function setResolutionUnit(next: ResolutionUnit) {
    resolutionUnit = next;
    rewrite(["resolution"]);
  }

  function setResample(next: boolean) {
    size = withResample(size, next);
    if (!next) sync();
  }

  function toggleConstrain() {
    size.constrain = !size.constrain;
    if (size.constrain) onSide("width");
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (valid) onapply(applied.width, applied.height, $state.snapshot(anchor), size.ppi);
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
    <header id="size-title" {@attach movable("size")}>
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
          class:on={size.constrain || !size.resample}
          aria-pressed={size.constrain || !size.resample}
          disabled={!size.resample}
          title={t("sizeDialog.constrain")}
          aria-label={t("sizeDialog.constrain")}
          onclick={toggleConstrain}
        >
          <Icon name={size.constrain || !size.resample ? "link" : "linkBroken"} size={14} />
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
        value={size.unit}
        onchange={(e) => setUnit((e.currentTarget as HTMLSelectElement).value as SizeUnit)}
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
            checked={size.resample}
            onchange={(e) => setResample((e.currentTarget as HTMLInputElement).checked)}
          />
          {t("sizeDialog.resample")}
        </label>
      {:else}
        <label class="check">
          <input
            type="checkbox"
            checked={size.relative}
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
        {t("sizeDialog.new", { width: applied.width, height: applied.height })}
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

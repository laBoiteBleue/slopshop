<script lang="ts">
  // The Properties panel (Photoshop's): the parameters of the selected adjustment layer
  // (ADR 0020), as sliders with a number field each. Dragging applies live, one undo entry per
  // drag; Reset puts the neutral values back.
  import type { AdjustmentId, EditRequest, LayerView } from "./engine";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";

  let {
    documentId,
    layer,
    onedit,
    onlive,
    ongestureend,
  }: {
    documentId: number;
    /** An adjustment layer. */
    layer: LayerView;
    onedit: (documentId: number, edit: EditRequest) => void;
    onlive: (documentId: number, edit: EditRequest) => void;
    ongestureend: (documentId: number) => void;
  } = $props();

  type Values = [number, number, number, number, number];
  /** One parameter: where it is in the values, its range as shown, and the stored value's
   * scale (Levels are shown 0–255, stored 0–1). */
  type Field = {
    index: number;
    label: MessageKey;
    min: number;
    max: number;
    step: number;
    scale: number;
  };

  const FIELDS: Record<AdjustmentId, Field[]> = {
    exposure: [
      { index: 0, label: "adjustment.exposure.exposure", min: -20, max: 20, step: 0.01, scale: 1 },
      {
        index: 1,
        label: "adjustment.exposure.offset",
        min: -0.5,
        max: 0.5,
        step: 0.0001,
        scale: 1,
      },
      { index: 2, label: "adjustment.exposure.gamma", min: 0.01, max: 9.99, step: 0.01, scale: 1 },
    ],
    hueSaturation: [
      { index: 0, label: "adjustment.hueSaturation.hue", min: -180, max: 180, step: 1, scale: 1 },
      {
        index: 1,
        label: "adjustment.hueSaturation.saturation",
        min: -100,
        max: 100,
        step: 1,
        scale: 1,
      },
      {
        index: 2,
        label: "adjustment.hueSaturation.lightness",
        min: -100,
        max: 100,
        step: 1,
        scale: 1,
      },
    ],
    levels: [
      { index: 0, label: "adjustment.levels.inputBlack", min: 0, max: 253, step: 1, scale: 255 },
      { index: 2, label: "adjustment.levels.gamma", min: 0.01, max: 9.99, step: 0.01, scale: 1 },
      { index: 1, label: "adjustment.levels.inputWhite", min: 2, max: 255, step: 1, scale: 255 },
      { index: 3, label: "adjustment.levels.outputBlack", min: 0, max: 255, step: 1, scale: 255 },
      { index: 4, label: "adjustment.levels.outputWhite", min: 0, max: 255, step: 1, scale: 255 },
    ],
  };

  const NEUTRAL: Record<AdjustmentId, Values> = {
    exposure: [0, 0, 1, 0, 0],
    hueSaturation: [0, 0, 0, 0, 0],
    levels: [0, 1, 1, 0, 1],
  };

  const adjustment = $derived(layer.adjustment);
  const fields = $derived(adjustment ? FIELDS[adjustment.id] : []);
  /** The values shown: the layer's, or the ones being dragged. */
  let values = $state<Values>([0, 0, 0, 0, 0]);
  let dragging = false;
  $effect(() => {
    const current = adjustment?.values;
    if (current && !dragging) values = [...current];
  });

  /** `values` with field `f` set to `shown` (as displayed), kept valid. */
  function withValue(f: Field, shown: number): Values {
    const next: Values = [...values];
    let v = Math.min(Math.max(shown, f.min), f.max) / f.scale;
    // Levels: the input black stays below the input white.
    if (adjustment?.id === "levels" && f.index === 0) v = Math.min(v, next[1] - 2 / 255);
    if (adjustment?.id === "levels" && f.index === 1) v = Math.max(v, next[0] + 2 / 255);
    next[f.index] = v;
    return next;
  }

  function request(next: Values): EditRequest | null {
    if (!adjustment) return null;
    return { kind: "setAdjustment", id: layer.id, adjustment: adjustment.id, values: next };
  }

  function onSlide(f: Field, shown: number) {
    dragging = true;
    values = withValue(f, shown);
    const edit = request(values);
    if (edit) onlive(documentId, edit);
  }

  function onSlideEnd() {
    dragging = false;
    ongestureend(documentId);
  }

  function onField(f: Field, shown: number) {
    if (!Number.isFinite(shown)) return;
    values = withValue(f, shown);
    const edit = request(values);
    if (edit) onedit(documentId, edit);
  }

  function reset() {
    if (!adjustment) return;
    const edit = request([...NEUTRAL[adjustment.id]]);
    if (edit) onedit(documentId, edit);
  }

  const shown = (f: Field) => {
    const v = values[f.index] * f.scale;
    return f.scale === 1 ? Math.round(v / f.step) * f.step : Math.round(v);
  };
  const digits = (f: Field) => Math.max(0, Math.ceil(-Math.log10(f.step)));
</script>

{#if adjustment}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="tabs"><span class="tab active">{t("properties.title")}</span></div>
    <div class="title">
      <span>{t(`adjustment.${adjustment.id}`)}</span>
      <button type="button" class="reset" onclick={reset}>{t("properties.reset")}</button>
    </div>
    <div class="fields">
      {#each fields as f (f.index)}
        <label class="label" for="property-{f.index}">{t(f.label)}</label>
        <input
          class="number"
          id="property-{f.index}"
          type="number"
          min={f.min}
          max={f.max}
          step={f.step}
          value={shown(f).toFixed(digits(f))}
          onchange={(e) => onField(f, Number((e.currentTarget as HTMLInputElement).value))}
        />
        <input
          class="slider"
          type="range"
          min={f.min}
          max={f.max}
          step={f.step}
          value={shown(f)}
          aria-label={t(f.label)}
          oninput={(e) => onSlide(f, Number((e.currentTarget as HTMLInputElement).value))}
          onchange={onSlideEnd}
        />
      {/each}
    </div>
  </section>
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
    border-top: 1px solid var(--border-dark);
  }

  .tabs {
    display: flex;
    height: 26px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
  }

  .tab {
    display: flex;
    align-items: center;
    padding: 0 12px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .tab.active {
    background: var(--panel);
    color: var(--text);
    border-right: 1px solid var(--border-dark);
  }

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .reset {
    font-weight: normal;
  }

  .fields {
    display: grid;
    grid-template-columns: 1fr 64px;
    align-items: center;
    gap: 2px 8px;
    padding: 4px 8px 10px;
  }

  .label {
    color: var(--text-muted);
  }

  .number {
    width: 64px;
    min-width: 0;
  }

  .slider {
    grid-column: 1 / -1;
    margin: 0 0 4px;
    accent-color: var(--accent);
  }
</style>

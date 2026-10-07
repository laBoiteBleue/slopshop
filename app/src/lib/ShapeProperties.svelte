<script lang="ts">
  // A vector layer's shape in the Properties panel (ADR 0041): its fill and stroke (a checkbox
  // and a swatch each, the swatch opening the color picker), the stroke's width and position,
  // a rectangle's corner radius, a polygon's sides and star indent. Each change is one undo
  // entry; where the shape lies is Free Transform's.
  import type { EditRequest, LayerView } from "./engine";
  import { t } from "./i18n/index.svelte";
  import {
    changedShape,
    hexOf,
    MAX_RADIUS,
    MAX_SIDES,
    MAX_STROKE_WIDTH,
    MIN_SIDES,
    type Shape,
    type ShapeChange,
    type StrokeAlign,
  } from "./shapes";

  let {
    documentId,
    layer,
    shape,
    onedit,
    onpickcolor,
  }: {
    documentId: number;
    layer: LayerView;
    shape: Shape;
    onedit: (documentId: number, edit: EditRequest) => void;
    /** A swatch was clicked: the app lets its color be chosen. */
    onpickcolor?: (layer: LayerView, which: "fill" | "stroke") => void;
  } = $props();

  const kind = $derived(shape.geometry.kind);
  const line = $derived(kind === "line");

  function change(c: ShapeChange) {
    onedit(documentId, { kind: "setShape", id: layer.id, shape: changedShape(shape, c) });
  }

  /** A number typed in a field, or null (the field shows the layer's again). */
  function typed(e: Event): number | null {
    const input = e.currentTarget as HTMLInputElement;
    const value = input.valueAsNumber;
    if (Number.isFinite(value)) return value;
    input.value = input.defaultValue;
    return null;
  }

  /** The color a fill or stroke turned on takes: the other one's, else black. */
  const otherColor = (color: [number, number, number, number] | null | undefined) =>
    color ? hexOf(color) : "#000000";
</script>

<section class="panel" aria-label={t("properties.title")}>
  <div class="title">
    <span>{t(`properties.shape.${kind}`)}</span>
  </div>
  {#if !line}
    <div class="row">
      <label>
        <input
          type="checkbox"
          checked={shape.fill !== null}
          onchange={(e) =>
            change({ fill: e.currentTarget.checked ? otherColor(shape.stroke?.color) : null })}
        />
        {t("options.shape.fill")}
      </label>
      {#if shape.fill}
        <button
          type="button"
          class="swatch"
          style:background={hexOf(shape.fill)}
          title={t("options.shape.fillColor")}
          aria-label={t("options.shape.fillColor")}
          onclick={() => onpickcolor?.(layer, "fill")}
        ></button>
      {/if}
    </div>
    <div class="row">
      <label>
        <input
          type="checkbox"
          checked={shape.stroke !== null}
          onchange={(e) =>
            change({ stroke: e.currentTarget.checked ? otherColor(shape.fill) : null })}
        />
        {t("options.shape.stroke")}
      </label>
      {#if shape.stroke}
        <button
          type="button"
          class="swatch"
          style:background={hexOf(shape.stroke.color)}
          title={t("options.shape.strokeColor")}
          aria-label={t("options.shape.strokeColor")}
          onclick={() => onpickcolor?.(layer, "stroke")}
        ></button>
      {/if}
    </div>
  {:else if shape.stroke}
    <div class="row">
      <span>{t("options.shape.stroke")}</span>
      <button
        type="button"
        class="swatch"
        style:background={hexOf(shape.stroke.color)}
        title={t("options.shape.strokeColor")}
        aria-label={t("options.shape.strokeColor")}
        onclick={() => onpickcolor?.(layer, "stroke")}
      ></button>
    </div>
  {/if}
  {#if shape.stroke}
    <div class="row">
      <label for="shape-width">{t("options.shape.width")}</label>
      <input
        id="shape-width"
        type="number"
        min="0.1"
        max={MAX_STROKE_WIDTH}
        step="0.1"
        value={shape.stroke.width}
        onchange={(e) => {
          const width = typed(e);
          if (width !== null) change({ strokeWidth: width });
        }}
      />
      <span class="unit">px</span>
    </div>
    {#if !line}
      <div class="row">
        <label for="shape-align">{t("options.shape.align")}</label>
        <select
          id="shape-align"
          value={shape.stroke.align}
          onchange={(e) => change({ strokeAlign: e.currentTarget.value as StrokeAlign })}
        >
          {#each ["inside", "center", "outside"] as const as align (align)}
            <option value={align}>{t(`options.shape.align.${align}`)}</option>
          {/each}
        </select>
      </div>
    {/if}
  {/if}
  {#if shape.geometry.kind === "rectangle"}
    <div class="row">
      <label for="shape-radius">{t("options.shape.radius")}</label>
      <input
        id="shape-radius"
        type="number"
        min="0"
        max={MAX_RADIUS}
        step="1"
        value={shape.geometry.radii[0]}
        onchange={(e) => {
          const radius = typed(e);
          if (radius !== null) change({ radius });
        }}
      />
      <span class="unit">px</span>
    </div>
  {:else if shape.geometry.kind === "polygon"}
    {@const star = shape.geometry.star}
    <div class="row">
      <label for="shape-sides">{t("options.shape.sides")}</label>
      <input
        id="shape-sides"
        type="number"
        min={MIN_SIDES}
        max={MAX_SIDES}
        step="1"
        value={shape.geometry.sides}
        onchange={(e) => {
          const sides = typed(e);
          if (sides !== null) change({ sides });
        }}
      />
    </div>
    <div class="row">
      <label>
        <input
          type="checkbox"
          checked={star !== null}
          onchange={(e) => change({ star: e.currentTarget.checked ? 0.5 : null })}
        />
        {t("options.shape.star")}
      </label>
    </div>
    {#if star !== null}
      <div class="row">
        <label for="shape-indent">{t("options.shape.starRatio")}</label>
        <input
          id="shape-indent"
          type="number"
          min="1"
          max="100"
          step="1"
          value={Math.round(star * 100)}
          onchange={(e) => {
            const percent = typed(e);
            if (percent !== null) change({ star: percent / 100 });
          }}
        />
        <span class="unit">%</span>
      </div>
    {/if}
  {/if}
</section>

<style>
  /* As the Properties panel's other sections. */
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
  }

  .title {
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
  }

  .row > label {
    min-width: 64px;
  }

  .row input[type="number"] {
    width: 64px;
  }

  .unit {
    color: var(--text-muted);
  }

  .swatch {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 2px;
  }
</style>

<script lang="ts">
  // A vector layer's shape in the Properties panel (ADR 0041): its fill and stroke (a checkbox
  // and a swatch each, the swatch opening the color picker), the stroke's width and position,
  // its bounds in document pixels (W, H, X, Y, `shapeBounds.ts`), a rectangle's corner radii
  // (linked or each its own), a polygon's sides and star indent. Each change is one undo entry.
  import type { EditRequest, LayerView, Matrix } from "./engine";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";
  import { documentBounds, withBounds, type BoundsField } from "./shapeBounds";
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
    matrix,
    onedit,
    onpickcolor,
  }: {
    documentId: number;
    layer: LayerView;
    shape: Shape;
    /** From the layer's space to the document's (its groups' too); null in perspective. */
    matrix: Matrix | null;
    onedit: (documentId: number, edit: EditRequest) => void;
    /** A swatch was clicked: the app lets its color be chosen. */
    onpickcolor?: (layer: LayerView, which: "fill" | "stroke") => void;
  } = $props();

  const kind = $derived(shape.geometry.kind);
  const line = $derived(kind === "line");

  function change(c: ShapeChange) {
    onedit(documentId, { kind: "setShape", id: layer.id, shape: changedShape(shape, c) });
  }

  /** W, H, X and Y; null where the layer rotates, skews or is in perspective. */
  const bounds = $derived(documentBounds(shape.geometry, matrix));
  const BOUNDS_FIELDS = $derived<{ key: BoundsField; label: string }[][]>([
    [
      { key: "width", label: t("transform.field.width") },
      { key: "height", label: t("transform.field.height") },
    ],
    [
      { key: "x", label: "X" },
      { key: "y", label: "Y" },
    ],
  ]);

  /** A value shown in a field: to the hundredth of a pixel. */
  const shown = (v: number) => Math.round(v * 100) / 100;

  function setBound(e: Event, key: BoundsField) {
    const value = typed(e);
    if (value === null) return;
    const geometry = withBounds(shape.geometry, matrix, key, value);
    if (!geometry) {
      const input = e.currentTarget as HTMLInputElement;
      input.value = input.defaultValue;
      return;
    }
    onedit(documentId, { kind: "setShape", id: layer.id, shape: { ...shape, geometry } });
  }

  /** The layer whose corners were unlinked although alike (each its own field). */
  let unlinkedFor = $state<number | null>(null);
  const radii = $derived(shape.geometry.kind === "rectangle" ? shape.geometry.radii : null);
  const linked = $derived(
    radii !== null && unlinkedFor !== layer.id && radii.every((r) => r === radii[0]),
  );
  const CORNERS = ["topLeft", "topRight", "bottomRight", "bottomLeft"] as const;

  function toggleLink() {
    if (!radii) return;
    if (linked) {
      unlinkedFor = layer.id;
      return;
    }
    unlinkedFor = null;
    // Linked again: every corner takes the first one's radius.
    if (!radii.every((r) => r === radii[0])) change({ radius: radii[0] });
  }

  function setCorner(e: Event, corner: number) {
    const value = typed(e);
    if (value === null || !radii) return;
    const next = [...radii] as [number, number, number, number];
    next[corner] = value;
    change({ radii: next });
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
  {#if bounds}
    {#each BOUNDS_FIELDS as row, i (i)}
      <div class="row">
        {#each row as field (field.key)}
          <label for={`shape-bounds-${field.key}`} class="short">{field.label}</label>
          <input
            id={`shape-bounds-${field.key}`}
            type="number"
            step="1"
            min={field.key === "width" || field.key === "height" ? 0 : undefined}
            value={shown(bounds[field.key])}
            onchange={(e) => setBound(e, field.key)}
          />
          <span class="unit">px</span>
        {/each}
      </div>
    {/each}
  {/if}
  {#if shape.geometry.kind === "rectangle" && radii}
    <div class="row">
      {#if linked}
        <label for="shape-radius">{t("options.shape.radius")}</label>
        <input
          id="shape-radius"
          type="number"
          min="0"
          max={MAX_RADIUS}
          step="1"
          value={radii[0]}
          onchange={(e) => {
            const radius = typed(e);
            if (radius !== null) change({ radius });
          }}
        />
        <span class="unit">px</span>
      {:else}
        <span class="corners-label">{t("properties.shape.corners")}</span>
      {/if}
      <button
        type="button"
        class="icon-btn"
        class:on={linked}
        title={t("properties.shape.linkCorners")}
        aria-label={t("properties.shape.linkCorners")}
        aria-pressed={linked}
        onmousedown={keepFocus}
        onclick={toggleLink}
      >
        <Icon name={linked ? "link" : "linkBroken"} size={14} />
      </button>
    </div>
    {#if !linked}
      <div class="corners">
        {#each CORNERS as corner, i (corner)}
          <input
            type="number"
            min="0"
            max={MAX_RADIUS}
            step="1"
            aria-label={t(`properties.shape.corner.${corner}`)}
            title={t(`properties.shape.corner.${corner}`)}
            value={radii[i]}
            onchange={(e) => setCorner(e, i)}
          />
        {/each}
      </div>
    {/if}
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

  .row > label.short {
    min-width: 0;
  }

  .corners-label {
    min-width: 64px;
  }

  /* The four radii as the corners they round: top left, top right, then bottom right and
     bottom left (clockwise, as Photoshop's). */
  .corners {
    display: grid;
    grid-template-columns: 64px 64px;
    grid-template-areas: "tl tr" "bl br";
    gap: 4px;
    padding: 4px 10px 4px 82px;
  }

  .corners input:nth-child(1) {
    grid-area: tl;
  }

  .corners input:nth-child(2) {
    grid-area: tr;
  }

  .corners input:nth-child(3) {
    grid-area: br;
  }

  .corners input:nth-child(4) {
    grid-area: bl;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
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

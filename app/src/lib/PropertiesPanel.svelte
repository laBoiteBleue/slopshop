<script lang="ts">
  // The Properties panel (Photoshop's), in the dock below Layers: the parameters of the selected adjustment layer
  // (ADR 0020), as sliders with a number field each, checkboxes and color swatches. Dragging
  // applies live, one undo entry per drag; Reset puts the neutral values back. For a fill
  // layer, its color: a swatch that opens the color picker. For a gradient fill layer, its
  // gradient, style, angle and scale (`gradientFill.ts`), and Reverse.
  import type { EditRequest, LayerView } from "./engine";
  import { fillHex } from "./layerEdits";
  import AdjustmentFields from "./AdjustmentFields.svelte";
  import GradientEditor from "./GradientEditor.svelte";
  import {
    fillPlacement,
    placed,
    reversed,
    type GradientFill,
    type GradientShape,
  } from "./gradientFill";
  import { t } from "./i18n/index.svelte";

  let {
    documentId,
    layer,
    size = { width: 1, height: 1 },
    onedit,
    onlive,
    ongestureend,
    onfillcolor,
  }: {
    documentId: number;
    /** An adjustment or a fill layer. */
    layer: LayerView;
    /** The document's size, where a gradient fill's angle and scale are measured. */
    size?: { width: number; height: number };
    /** The fill layer's swatch was clicked: the app lets its color be chosen. */
    onfillcolor?: (layer: LayerView) => void;
    onedit: (documentId: number, edit: EditRequest) => void;
    onlive: (documentId: number, edit: EditRequest) => void;
    ongestureend: (documentId: number) => void;
  } = $props();

  const adjustment = $derived(layer.adjustment);
  let fields = $state<AdjustmentFields | null>(null);

  /** The edit with these settings (Gradient Map: its stops, these or the layer's). */
  function request(values: number[], gradient?: number[][]): EditRequest | null {
    if (!adjustment) return null;
    const stops = gradient ?? adjustment.gradient ?? undefined;
    return {
      kind: "setAdjustment",
      id: layer.id,
      adjustment: adjustment.id,
      values,
      ...(stops ? { gradient: stops } : {}),
    };
  }

  /** Curves: the edit with these points (composite, red, green, blue). */
  function curvesRequest(curves: number[][][]): EditRequest {
    return { kind: "setAdjustment", id: layer.id, adjustment: "curves", values: [], curves };
  }

  function live(values: number[], gradient?: number[][]) {
    const edit = request(values, gradient);
    if (edit) onlive(documentId, edit);
  }

  function apply(values: number[], gradient?: number[][]) {
    const edit = request(values, gradient);
    if (edit) onedit(documentId, edit);
  }

  const fill = $derived(layer.gradientFill ?? null);
  const placement = $derived(fill ? fillPlacement(size, fill) : null);

  const fillRequest = (gradient: GradientFill): EditRequest => ({
    kind: "setGradientFill",
    id: layer.id,
    gradient,
  });

  /** A number typed in a field, or null (the field shows the layer's again). */
  function typed(e: Event): number | null {
    const input = e.currentTarget as HTMLInputElement;
    const value = input.valueAsNumber;
    if (Number.isFinite(value)) return value;
    input.value = input.defaultValue;
    return null;
  }

  function setAngle(e: Event) {
    const angle = typed(e);
    if (fill && angle !== null) onedit(documentId, fillRequest(placed(size, fill, { angle })));
  }

  function setScale(e: Event) {
    const percent = typed(e);
    if (!fill || percent === null) return;
    const scale = Math.min(Math.max(percent, 1), 1000) / 100;
    onedit(documentId, fillRequest(placed(size, fill, { scale })));
  }
</script>

{#if layer.kind === "fill"}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="title"><span>{t("menu.layer.newFill.solidColor")}</span></div>
    <label class="fill">
      <span>{t("properties.fillColor")}</span>
      <button
        type="button"
        class="swatch"
        style:background={fillHex(layer)}
        aria-label={t("colorPicker.fill")}
        title={t("colorPicker.fill")}
        onclick={() => onfillcolor?.(layer)}
      ></button>
    </label>
  </section>
{:else if layer.kind === "gradientFill" && fill && placement}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="title">
      <span>{t("menu.layer.newFill.gradient")}</span>
      <button
        type="button"
        class="btn small"
        onclick={() => onedit(documentId, fillRequest(reversed(fill)))}
      >
        {t("options.gradient.reverse")}
      </button>
    </div>
    <GradientEditor
      stops={fill.stops}
      onlive={(stops) => onlive(documentId, fillRequest({ ...fill, stops }))}
      onend={() => ongestureend(documentId)}
      onapply={(stops) => onedit(documentId, fillRequest({ ...fill, stops }))}
    />
    <div class="row">
      <label for="gradient-fill-shape">{t("properties.gradientShape")}</label>
      <select
        id="gradient-fill-shape"
        value={fill.shape}
        onchange={(e) => {
          const shape = e.currentTarget.value as GradientShape;
          onedit(documentId, fillRequest(placed(size, fill, { shape })));
        }}
      >
        <option value="linear">{t("options.gradient.linear")}</option>
        <option value="radial">{t("options.gradient.radial")}</option>
      </select>
    </div>
    <div class="row">
      <label for="gradient-fill-angle">{t("properties.gradientAngle")}</label>
      <input
        id="gradient-fill-angle"
        type="number"
        step="1"
        value={Math.round(placement.angle)}
        onchange={setAngle}
      />
      <span class="unit">°</span>
    </div>
    <div class="row">
      <label for="gradient-fill-scale">{t("properties.gradientScale")}</label>
      <input
        id="gradient-fill-scale"
        type="number"
        min="1"
        max="1000"
        step="1"
        value={Math.round(placement.scale * 100)}
        onchange={setScale}
      />
      <span class="unit">%</span>
    </div>
  </section>
{:else if adjustment}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="title">
      <span>{t(`adjustment.${adjustment.id}`)}</span>
      {#if fields?.resettable()}
        <button type="button" class="btn small" onclick={() => fields?.reset()}>
          {t("properties.reset")}
        </button>
      {/if}
    </div>
    {#if !fields?.resettable()}
      <p class="empty">{t("properties.noSettings")}</p>
    {/if}
    <AdjustmentFields
      bind:this={fields}
      {adjustment}
      owner={layer.id}
      onlive={live}
      onapply={apply}
      onend={() => ongestureend(documentId)}
      oncurveslive={(curves) => onlive(documentId, curvesRequest(curves))}
      oncurvesapply={(curves) => onedit(documentId, curvesRequest(curves))}
    />
  </section>
{/if}

<style>
  /* In the dock, which gives the tab and the border. */
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
  }

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .empty {
    margin: 0;
    padding: 4px 8px 10px;
    color: var(--text-muted);
  }

  .fill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
  }

  .row > label {
    width: 64px;
  }

  .row input {
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

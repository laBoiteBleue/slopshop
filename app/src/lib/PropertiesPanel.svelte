<script lang="ts">
  // The Properties panel (Photoshop's): the parameters of the selected adjustment layer
  // (ADR 0020), as sliders with a number field each, checkboxes and color swatches. Dragging
  // applies live, one undo entry per drag; Reset puts the neutral values back.
  import { ADJUSTMENT_PARAMS, type AdjustmentId, type EditRequest, type LayerView } from "./engine";
  import { hexToSrgb, srgbToHex } from "./color";
  import CurvesEditor from "./CurvesEditor.svelte";
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

  /** All the parameters (`ADJUSTMENT_PARAMS` of them, `Adjustment::params` order). */
  type Values = number[];
  /** A parameter as a slider: where it is in the values, its range as shown, and the stored
   * value's scale (Levels are shown 0–255, stored 0–1). `selected`: the index is relative to
   * the adjustment's selector (e.g. Color Balance's tone). `enabledBy`: a flag that must be on
   * (e.g. Black & White's tint). */
  type Slider = {
    kind: "slider";
    index: number;
    label: MessageKey;
    min: number;
    max: number;
    step: number;
    scale: number;
    selected?: boolean;
    enabledBy?: number;
  };
  /** A flag (0 or 1) as a checkbox. */
  type Check = { kind: "check"; index: number; label: MessageKey };
  /** Three sRGB-encoded values from `index`, as a color swatch. */
  type Color = { kind: "color"; index: number; label: MessageKey };
  type Field = Slider | Check | Color;
  /** Which parameters the `selected` sliders edit (Photoshop's Tone and Output Channel menus):
   * an offset into the values per option. Hidden (offset 0) while flag `hiddenBy` is on. */
  type Selector = {
    label: MessageKey;
    options: { label: MessageKey; offset: number }[];
    hiddenBy?: number;
  };

  const slider = (
    index: number,
    label: MessageKey,
    min: number,
    max: number,
    extra: Partial<Slider> = {},
  ): Slider => ({ kind: "slider", index, label, min, max, step: 1, scale: 1, ...extra });

  const FIELDS: Record<AdjustmentId, Field[]> = {
    exposure: [
      slider(0, "adjustment.exposure.exposure", -20, 20, { step: 0.01 }),
      slider(1, "adjustment.exposure.offset", -0.5, 0.5, { step: 0.0001 }),
      slider(2, "adjustment.exposure.gamma", 0.01, 9.99, { step: 0.01 }),
    ],
    hueSaturation: [
      slider(0, "adjustment.hueSaturation.hue", -180, 180),
      slider(1, "adjustment.hueSaturation.saturation", -100, 100),
      slider(2, "adjustment.hueSaturation.lightness", -100, 100),
    ],
    levels: [
      slider(0, "adjustment.levels.inputBlack", 0, 253, { scale: 255 }),
      slider(2, "adjustment.levels.gamma", 0.01, 9.99, { step: 0.01 }),
      slider(1, "adjustment.levels.inputWhite", 2, 255, { scale: 255 }),
      slider(3, "adjustment.levels.outputBlack", 0, 255, { scale: 255 }),
      slider(4, "adjustment.levels.outputWhite", 0, 255, { scale: 255 }),
    ],
    brightnessContrast: [
      slider(0, "adjustment.brightnessContrast.brightness", -150, 150),
      slider(1, "adjustment.brightnessContrast.contrast", -50, 100),
    ],
    vibrance: [
      slider(0, "adjustment.vibrance.vibrance", -100, 100),
      slider(1, "adjustment.vibrance.saturation", -100, 100),
    ],
    colorBalance: [
      slider(0, "adjustment.colorBalance.cyanRed", -100, 100, { selected: true }),
      slider(1, "adjustment.colorBalance.magentaGreen", -100, 100, { selected: true }),
      slider(2, "adjustment.colorBalance.yellowBlue", -100, 100, { selected: true }),
      { kind: "check", index: 9, label: "adjustment.preserveLuminosity" },
    ],
    blackWhite: [
      slider(0, "adjustment.blackWhite.reds", -200, 300),
      slider(1, "adjustment.blackWhite.yellows", -200, 300),
      slider(2, "adjustment.blackWhite.greens", -200, 300),
      slider(3, "adjustment.blackWhite.cyans", -200, 300),
      slider(4, "adjustment.blackWhite.blues", -200, 300),
      slider(5, "adjustment.blackWhite.magentas", -200, 300),
      { kind: "check", index: 6, label: "adjustment.blackWhite.tint" },
      slider(7, "adjustment.blackWhite.tintHue", 0, 360, { enabledBy: 6 }),
      slider(8, "adjustment.blackWhite.tintSaturation", 0, 100, { enabledBy: 6 }),
    ],
    photoFilter: [
      { kind: "color", index: 0, label: "adjustment.photoFilter.color" },
      slider(3, "adjustment.photoFilter.density", 0, 100),
      { kind: "check", index: 4, label: "adjustment.preserveLuminosity" },
    ],
    channelMixer: [
      slider(0, "adjustment.channelMixer.red", -200, 200, { selected: true }),
      slider(1, "adjustment.channelMixer.green", -200, 200, { selected: true }),
      slider(2, "adjustment.channelMixer.blue", -200, 200, { selected: true }),
      slider(3, "adjustment.channelMixer.constant", -200, 200, { selected: true }),
      { kind: "check", index: 12, label: "adjustment.channelMixer.monochrome" },
    ],
    invert: [],
    curves: [],
    posterize: [slider(0, "adjustment.posterize.levels", 2, 255)],
    threshold: [slider(0, "adjustment.threshold.level", 1, 255, { scale: 255 })],
  };

  const SELECTORS: Partial<Record<AdjustmentId, Selector>> = {
    colorBalance: {
      label: "adjustment.colorBalance.tone",
      options: [
        { label: "adjustment.colorBalance.shadows", offset: 0 },
        { label: "adjustment.colorBalance.midtones", offset: 3 },
        { label: "adjustment.colorBalance.highlights", offset: 6 },
      ],
    },
    channelMixer: {
      label: "adjustment.channelMixer.output",
      options: [
        { label: "adjustment.channelMixer.red", offset: 0 },
        { label: "adjustment.channelMixer.green", offset: 4 },
        { label: "adjustment.channelMixer.blue", offset: 8 },
      ],
      hiddenBy: 12,
    },
  };

  /** `values` filled with zeros up to `ADJUSTMENT_PARAMS`. */
  const padded = (values: readonly number[]): Values => [
    ...values,
    ...Array<number>(Math.max(0, ADJUSTMENT_PARAMS - values.length)).fill(0),
  ];

  /** What Reset puts back: the parameters of a new layer (Adjustment::DEFAULTS in core). */
  const DEFAULTS: Record<AdjustmentId, Values> = {
    exposure: padded([0, 0, 1]),
    hueSaturation: padded([]),
    levels: padded([0, 1, 1, 0, 1]),
    brightnessContrast: padded([]),
    vibrance: padded([]),
    colorBalance: padded([0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
    blackWhite: padded([40, 60, 40, 60, 20, 80, 0, 42, 20]),
    photoFilter: padded([236 / 255, 138 / 255, 0, 25, 1]),
    channelMixer: padded([100, 0, 0, 0, 0, 100, 0, 0, 0, 0, 100, 0, 0]),
    invert: padded([]),
    curves: padded([]),
    posterize: padded([4]),
    threshold: padded([128 / 255]),
  };

  const adjustment = $derived(layer.adjustment);
  const fields = $derived(adjustment ? FIELDS[adjustment.id] : []);
  const selector = $derived(adjustment ? SELECTORS[adjustment.id] : undefined);
  /** The values shown: the layer's, or the ones being dragged. */
  let values = $state<Values>(padded([]));
  let dragging = false;
  $effect(() => {
    const current = adjustment?.values;
    if (current && !dragging) values = padded(current);
  });
  /** The selector's choice; back to the first option for another layer. */
  let chosen = $state(0);
  $effect(() => {
    void layer.id;
    chosen = 0;
  });
  const selectorHidden = $derived(
    selector?.hiddenBy !== undefined && values[selector.hiddenBy] !== 0,
  );
  const offset = $derived(selector && !selectorHidden ? selector.options[chosen].offset : 0);

  /** Where slider `f` is in the values. */
  const at = (f: Slider) => (f.selected ? f.index + offset : f.index);

  /** `values` with slider `f` set to `shown` (as displayed), kept valid. */
  function withValue(f: Slider, shown: number): Values {
    const next = [...values];
    const i = at(f);
    let v = Math.min(Math.max(shown, f.min), f.max) / f.scale;
    // Levels: the input black stays below the input white.
    if (adjustment?.id === "levels" && i === 0) v = Math.min(v, next[1] - 2 / 255);
    if (adjustment?.id === "levels" && i === 1) v = Math.max(v, next[0] + 2 / 255);
    next[i] = v;
    return next;
  }

  function request(next: Values): EditRequest | null {
    if (!adjustment) return null;
    return { kind: "setAdjustment", id: layer.id, adjustment: adjustment.id, values: next };
  }

  /** Curves: the edit with these points (composite, red, green, blue). */
  function curvesRequest(curves: number[][][]): EditRequest {
    return { kind: "setAdjustment", id: layer.id, adjustment: "curves", values: [], curves };
  }

  const IDENTITY_CURVES = [0, 1, 2, 3].map(() => [
    [0, 0],
    [255, 255],
  ]);

  /** Show `next` and apply it live (part of a gesture). */
  function live(next: Values) {
    dragging = true;
    values = next;
    const edit = request(next);
    if (edit) onlive(documentId, edit);
  }

  function endGesture() {
    dragging = false;
    ongestureend(documentId);
  }

  /** Show `next` and apply it as one undoable edit. */
  function apply(next: Values) {
    values = next;
    const edit = request(next);
    if (edit) onedit(documentId, edit);
  }

  function onField(f: Slider, shown: number) {
    if (Number.isFinite(shown)) apply(withValue(f, shown));
  }

  function onCheck(f: Check, checked: boolean) {
    const next = [...values];
    next[f.index] = checked ? 1 : 0;
    apply(next);
  }

  function onColor(f: Color, hex: string) {
    const next = [...values];
    next.splice(f.index, 3, ...hexToSrgb(hex));
    live(next);
  }

  function reset() {
    if (adjustment?.id === "curves") onedit(documentId, curvesRequest(IDENTITY_CURVES));
    else if (adjustment) apply([...DEFAULTS[adjustment.id]]);
  }

  const shown = (f: Slider) => {
    const v = values[at(f)] * f.scale;
    return f.scale === 1 ? Math.round(v / f.step) * f.step : Math.round(v);
  };
  const digits = (f: Slider) => Math.max(0, Math.ceil(-Math.log10(f.step)));
  const disabled = (f: Slider) => f.enabledBy !== undefined && values[f.enabledBy] === 0;
</script>

{#if adjustment}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="tabs"><span class="tab active">{t("properties.title")}</span></div>
    <div class="title">
      <span>{t(`adjustment.${adjustment.id}`)}</span>
      {#if fields.length > 0 || adjustment.curves}
        <button type="button" class="btn small" onclick={reset}>{t("properties.reset")}</button>
      {/if}
    </div>
    {#if fields.length === 0 && !adjustment.curves}
      <p class="empty">{t("properties.noSettings")}</p>
    {/if}
    <div class="fields">
      {#if adjustment.curves && adjustment.curveSamples}
        <CurvesEditor
          curves={adjustment.curves}
          samples={adjustment.curveSamples}
          onlive={(curves) => onlive(documentId, curvesRequest(curves))}
          onend={() => ongestureend(documentId)}
          onapply={(curves) => onedit(documentId, curvesRequest(curves))}
        />
      {/if}
      {#if selector && !selectorHidden}
        <label class="label" for="property-selector">{t(selector.label)}</label>
        <select id="property-selector" class="selector" bind:value={chosen}>
          {#each selector.options as option, i (option.offset)}
            <option value={i}>{t(option.label)}</option>
          {/each}
        </select>
      {/if}
      {#each fields as f (f.index)}
        {#if f.kind === "slider"}
          <label class="label" class:dim={disabled(f)} for="property-{f.index}">{t(f.label)}</label>
          <input
            class="number"
            id="property-{f.index}"
            type="number"
            min={f.min}
            max={f.max}
            step={f.step}
            disabled={disabled(f)}
            value={shown(f).toFixed(digits(f))}
            onchange={(e) => onField(f, Number((e.currentTarget as HTMLInputElement).value))}
          />
          <input
            class="slider"
            type="range"
            min={f.min}
            max={f.max}
            step={f.step}
            disabled={disabled(f)}
            value={shown(f)}
            aria-label={t(f.label)}
            oninput={(e) => live(withValue(f, Number((e.currentTarget as HTMLInputElement).value)))}
            onchange={endGesture}
          />
        {:else if f.kind === "check"}
          <label class="check">
            <input
              type="checkbox"
              checked={values[f.index] !== 0}
              onchange={(e) => onCheck(f, (e.currentTarget as HTMLInputElement).checked)}
            />
            {t(f.label)}
          </label>
        {:else}
          <label class="label" for="property-{f.index}">{t(f.label)}</label>
          <input
            class="swatch"
            id="property-{f.index}"
            type="color"
            value={srgbToHex(values.slice(f.index, f.index + 3))}
            oninput={(e) => onColor(f, (e.currentTarget as HTMLInputElement).value)}
            onchange={endGesture}
          />
        {/if}
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

  .empty {
    margin: 0;
    padding: 4px 8px 10px;
    color: var(--text-muted);
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

  .label.dim {
    opacity: 0.5;
  }

  .number,
  .selector,
  .swatch {
    width: 64px;
    min-width: 0;
  }

  .selector {
    width: auto;
    min-width: 64px;
  }

  .slider {
    grid-column: 1 / -1;
    margin: 0 0 4px;
    accent-color: var(--accent);
  }

  .check {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 2px 0 4px;
  }
</style>

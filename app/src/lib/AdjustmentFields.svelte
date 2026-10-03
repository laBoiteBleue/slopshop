<script lang="ts">
  // The settings of an adjustment (ADR 0020), as Photoshop shows them: sliders with a number
  // field each, checkboxes, color swatches, menus choosing what the sliders edit, and the Curves
  // editor. Used by the Properties panel (an adjustment layer) and by Image > Adjustments.
  import { ADJUSTMENT_PARAMS, type AdjustmentId, type LayerView } from "./engine";
  import { hexToSrgb, srgbToHex } from "./color";
  import CurvesEditor from "./CurvesEditor.svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";

  let {
    adjustment,
    owner,
    onlive,
    onapply,
    onend,
    oncurveslive,
    oncurvesapply,
  }: {
    /** The adjustment and its settings, as the engine has them. */
    adjustment: NonNullable<LayerView["adjustment"]>;
    /** What the settings belong to (e.g. a layer id): the menus go back to their first
     * option when it changes. */
    owner: number;
    /** Settings changed during a drag (part of a gesture). */
    onlive: (values: number[]) => void;
    /** Settings changed at once (one change). */
    onapply: (values: number[]) => void;
    /** The end of a drag. */
    onend: () => void;
    /** Curves' points changed during a drag, or at once: composite, red, green, blue. */
    oncurveslive: (curves: number[][][]) => void;
    oncurvesapply: (curves: number[][][]) => void;
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
      slider(0, "adjustment.levels.inputBlack", 0, 253, { scale: 255, selected: true }),
      slider(2, "adjustment.levels.gamma", 0.01, 9.99, { step: 0.01, selected: true }),
      slider(1, "adjustment.levels.inputWhite", 2, 255, { scale: 255, selected: true }),
      slider(3, "adjustment.levels.outputBlack", 0, 255, { scale: 255, selected: true }),
      slider(4, "adjustment.levels.outputWhite", 0, 255, { scale: 255, selected: true }),
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
    // Photoshop's Channel menu: the composite, then each channel's own settings.
    levels: {
      label: "adjustment.levels.channel",
      options: [
        { label: "adjustment.levels.rgb", offset: 0 },
        { label: "adjustment.levels.red", offset: 5 },
        { label: "adjustment.levels.green", offset: 10 },
        { label: "adjustment.levels.blue", offset: 15 },
      ],
    },
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
    levels: padded([0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1]),
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

  const fields = $derived(FIELDS[adjustment.id]);
  const selector = $derived(SELECTORS[adjustment.id]);
  /** The values shown: the adjustment's, or the ones being dragged. */
  let values = $state<Values>(padded([]));
  let dragging = false;
  $effect(() => {
    const current = adjustment.values;
    if (!dragging) values = padded(current);
  });
  /** The selector's choice; back to the first option for another owner. */
  let chosen = $state(0);
  $effect(() => {
    void owner;
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
    // Levels: the input black stays below the input white (of the channel shown).
    if (adjustment.id === "levels" && f.index === 0) v = Math.min(v, next[i + 1] - 2 / 255);
    if (adjustment.id === "levels" && f.index === 1) v = Math.max(v, next[i - 1] + 2 / 255);
    next[i] = v;
    return next;
  }

  const IDENTITY_CURVES = [0, 1, 2, 3].map(() => [
    [0, 0],
    [255, 255],
  ]);

  /** Show `next` and apply it live (part of a gesture). */
  function live(next: Values) {
    dragging = true;
    values = next;
    onlive(next);
  }

  function endGesture() {
    dragging = false;
    onend();
  }

  /** Show `next` and apply it as one change. */
  function apply(next: Values) {
    values = next;
    onapply(next);
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

  /** Whether the adjustment has settings to reset. */
  export function resettable(): boolean {
    return fields.length > 0 || adjustment.curves !== null;
  }

  /** Put the neutral settings back (those of a new adjustment layer). */
  export function reset() {
    if (adjustment.id === "curves") oncurvesapply(IDENTITY_CURVES);
    else apply([...DEFAULTS[adjustment.id]]);
  }

  const shown = (f: Slider) => {
    const v = values[at(f)] * f.scale;
    return f.scale === 1 ? Math.round(v / f.step) * f.step : Math.round(v);
  };
  const digits = (f: Slider) => Math.max(0, Math.ceil(-Math.log10(f.step)));
  const disabled = (f: Slider) => f.enabledBy !== undefined && values[f.enabledBy] === 0;
</script>

<div class="fields">
  {#if adjustment.curves && adjustment.curveSamples}
    <CurvesEditor
      curves={adjustment.curves}
      samples={adjustment.curveSamples}
      onlive={oncurveslive}
      {onend}
      onapply={oncurvesapply}
    />
  {/if}
  {#if selector && !selectorHidden}
    <!-- A row of its own: the options' names (Midtones…) need more than a number's width. -->
    <div class="selector-row">
      <label class="label" for="property-selector">{t(selector.label)}</label>
      <select id="property-selector" class="selector" bind:value={chosen}>
        {#each selector.options as option, i (option.offset)}
          <option value={i}>{t(option.label)}</option>
        {/each}
      </select>
    </div>
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

<style>
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
  .swatch {
    width: 64px;
    min-width: 0;
  }

  .selector-row {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }

  .selector {
    flex: 1;
    min-width: 0;
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

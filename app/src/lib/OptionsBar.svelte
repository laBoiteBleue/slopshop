<script lang="ts">
  // The options bar (ADR 0013): the settings of the active tool, under the menu bar. Only what
  // belongs to the tool: view and apply/cancel commands live in the menus and on the keys.
  import type { SelectionMode, ToneRange } from "./engine";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import { MAX_BRUSH, MAX_FEATHER } from "./selection";
  import { isSelectionTool, toolInfo, type ToolId } from "./tools";
  import { keepFocus } from "./platform";
  import SliderField from "./SliderField.svelte";
  import BrushPicker from "./BrushPicker.svelte";
  import type { Snippet } from "svelte";
  import { ALIGNS, DISTRIBUTES, type AlignId, type DistributeId } from "./align";
  import { CROP_RATIOS, type CropAspect } from "./crop";
  import type { ToolGradient } from "./gradient";
  import {
    defaultShapeOptions,
    MAX_RADIUS,
    MAX_SIDES,
    MAX_STROKE_WIDTH,
    MIN_SIDES,
    shapeKindOf,
    type ShapeOptions,
  } from "./shapes";

  let {
    tool,
    autoSelect = $bindable(),
    selectionMode = $bindable(),
    feather = $bindable(),
    antiAlias = $bindable(),
    wand = $bindable(),
    quick = $bindable(),
    brush = $bindable(),
    eraser = $bindable(),
    focus = $bindable({
      size: 30,
      hardness: 0,
      opacity: 1,
      flow: 1,
      pressureSize: true,
      pressureOpacity: false,
      strength: 0.5,
    }),
    tone = $bindable({
      size: 60,
      hardness: 0,
      opacity: 1,
      flow: 1,
      pressureSize: true,
      pressureOpacity: false,
      range: "midtones",
      exposure: 0.5,
    }),
    heal = $bindable({
      mode: "brush",
      size: 30,
      hardness: 1,
      opacity: 1,
      flow: 1,
      pressureSize: true,
      pressureOpacity: false,
      aligned: true,
      sample: "layer",
    }),
    clone = $bindable({
      size: 30,
      hardness: 0.5,
      opacity: 1,
      flow: 1,
      pressureSize: true,
      pressureOpacity: false,
      aligned: true,
      sample: "layer",
    }),
    eyedropper = $bindable({ sample: "all", size: 1 }),
    gradient = $bindable({
      preset: "foregroundToBackground",
      shape: "linear",
      reverse: false,
      opacity: 1,
    }),
    bucket = $bindable({
      opacity: 1,
      tolerance: 32,
      contiguous: true,
      antiAlias: true,
      sampleAll: false,
    }),
    crop = $bindable({ mode: "free" }),
    straighten = $bindable(false),
    canvasSize = { width: 1, height: 1 },
    transform,
    quickMask = false,
    quickMaskOpacity = $bindable(50),
    alignable = false,
    distributable = false,
    onalign,
    ondistribute,
    shape = $bindable(defaultShapeOptions("#000000")),
    onpickcolor,
  }: {
    tool: ToolId;
    /** Free Transform under way: its fields replace the tool's options, as in Photoshop. */
    transform?: Snippet;
    /** Move tool: a drag takes the layer under the pointer (Ctrl inverts it). */
    autoSelect: boolean;
    /** Move tool: the selected layers can be aligned (one at least), distributed (three). */
    alignable?: boolean;
    distributable?: boolean;
    /** Move tool's buttons: Layer > Align and Distribute on the selected layers. */
    onalign?: (align: AlignId) => void;
    ondistribute?: (distribute: DistributeId) => void;
    /** Shape tools (ADR 0041): the fill and stroke of the next shape, and its geometry's. */
    shape?: ShapeOptions;
    /** Shape tools: a swatch was clicked; the color picker sets `shape.fill` or `shape.stroke`. */
    onpickcolor?: (which: "fill" | "stroke") => void;
    /** Selection tools: how a new shape combines with the selection (keys override it). */
    selectionMode: SelectionMode;
    /** Selection tools: Gaussian softening of the edge, in pixels. */
    feather: number;
    /** Elliptical Marquee, lassos and Magic Wand: smooth edges. */
    antiAlias: boolean;
    /** Magic Wand: tolerance (0–255), connected pixels only, every layer or the active one. */
    wand: { tolerance: number; contiguous: boolean; sampleAll: boolean };
    /** Object and Quick Selection: the brush diameter (document pixels); every layer or the active one. */
    quick: { size: number; sampleAll: boolean; objectRefine: boolean };
    /** Brush and Eraser (ADR 0027): size in document pixels, the rest as shares in [0, 1]. */
    brush: PaintOptions;
    eraser: PaintOptions;
    /** Clone Stamp: its brush, Aligned, and what it samples (Photoshop's Current Layer first). */
    clone?: PaintOptions & { aligned: boolean; sample: "all" | "layer" };
    /** Blur and Sharpen: their brush and strength. */
    focus?: PaintOptions & { strength: number };
    /** Dodge and Burn: their brush, the range of tones and the exposure. */
    tone?: PaintOptions & { range: ToneRange; exposure: number };
    /** Healing Brush: as the Clone Stamp's. */
    heal?: PaintOptions & { mode: "brush" | "patch"; aligned: boolean; sample: "all" | "layer" };
    /** Eyedropper: every visible layer or the active one alone, and the side of the average. */
    eyedropper?: { sample: "all" | "layer"; size: number };
    /** Gradient: which gradient, its shape, reversed or not, and its opacity. */
    gradient?: {
      preset: ToolGradient;
      shape: "linear" | "radial";
      reverse: boolean;
      opacity: number;
    };
    /** Paint Bucket: the fill's opacity, then the Magic Wand's region (every layer or the active one). */
    bucket?: {
      opacity: number;
      tolerance: number;
      contiguous: boolean;
      antiAlias: boolean;
      sampleAll: boolean;
    };
    /** Crop: the frame's ratio or size (pixels), if any. */
    crop?: CropAspect;
    /** Crop: a drag draws a line along what should be level, and the image turns. */
    straighten?: boolean;
    /** The document's size: Crop's Original Ratio, and the size it starts from. */
    canvasSize?: { width: number; height: number };
    /** Quick Mask is on (whatever the tool): said, with its overlay's opacity. */
    quickMask?: boolean;
    /** Quick Mask's overlay opacity, percent. */
    quickMaskOpacity?: number;
  } = $props();

  type PaintOptions = {
    size: number;
    hardness: number;
    opacity: number;
    flow: number;
    pressureSize: boolean;
    pressureOpacity: boolean;
    /** Brush and Eraser: Photoshop's Pencil (hard pixels). */
    pencil?: boolean;
  };

  /** The painting tool's options, edited in place. */
  const paint = $derived(
    tool === "eraser" || tool === "restoreEraser"
      ? eraser
      : tool === "brush"
        ? brush
        : tool === "cloneStamp"
          ? clone
          : tool === "healingBrush"
            ? heal
            : tool === "dodge" || tool === "burn"
              ? tone
              : tool === "blur" || tool === "sharpen" || tool === "smudge"
                ? focus
                : null,
  );

  const current = $derived(toolInfo(tool));

  /** Crop's choice: free, a preset ratio (`w:h`), the canvas's, a ratio typed, or a size. */
  function cropPreset(aspect: CropAspect): string {
    if (aspect.mode !== "ratio") return aspect.mode;
    const preset = CROP_RATIOS.find(([w, h]) => w === aspect.width && h === aspect.height);
    if (preset) return `${preset[0]}:${preset[1]}`;
    const original = canvasSize.width / canvasSize.height;
    return Math.abs(aspect.width / aspect.height - original) < 1e-9 ? "original" : "ratio";
  }

  function chooseCropPreset(value: string) {
    if (value === "free") crop = { mode: "free" };
    else if (value === "size") crop = { mode: "size", ...canvasSize };
    else if (value === "original") crop = { mode: "ratio", ...canvasSize };
    else if (value === "ratio") crop = { mode: "ratio", width: 1, height: 1 };
    else {
      const [width, height] = value.split(":").map(Number);
      crop = { mode: "ratio", width, height };
    }
  }

  /** Photoshop's Sample Sizes: a pixel, or the average of a square around it. */
  const SAMPLE_SIZES = [1, 3, 5, 11, 31, 51, 101];

  const MODES: { mode: SelectionMode; icon: IconName; label: MessageKey }[] = [
    { mode: "replace", icon: "selectionReplace", label: "options.mode.replace" },
    { mode: "add", icon: "selectionAdd", label: "options.mode.add" },
    { mode: "subtract", icon: "selectionSubtract", label: "options.mode.subtract" },
    { mode: "intersect", icon: "selectionIntersect", label: "options.mode.intersect" },
  ];
</script>

<div class="options" role="toolbar" aria-label={t("options.label")}>
  <span class="tool-icon" title={t(current.name)}>
    <Icon name={current.icon} size={16} />
  </span>
  <span class="divider"></span>
  {#if quickMask}
    <!-- A mode, not a tool: shown whatever the tool, so that it is never forgotten. -->
    <span class="mode" role="status">{t("quickMask.label")}</span>
    <SliderField
      label={t("quickMask.opacity")}
      bind:value={quickMaskOpacity}
      min={0}
      max={100}
      unit="%"
      width={44}
    />
    <span class="divider"></span>
  {/if}

  {#if transform}
    {@render transform()}
  {:else if tool === "move"}
    <label class="option">
      <input type="checkbox" bind:checked={autoSelect} />
      {t("options.autoSelect")}
    </label>
    <span class="divider"></span>
    <!-- Layer > Align and Distribute, as Photoshop shows them here. -->
    {#each ALIGNS as entry (entry.id)}
      <button
        class="icon-btn"
        onmousedown={keepFocus}
        title={t(entry.hint)}
        aria-label={t(entry.hint)}
        disabled={!alignable}
        onclick={() => onalign?.(entry.id)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
    <span class="divider"></span>
    {#each DISTRIBUTES as entry (entry.id)}
      <button
        class="icon-btn"
        onmousedown={keepFocus}
        title={t(entry.hint)}
        aria-label={t(entry.hint)}
        disabled={!distributable}
        onclick={() => ondistribute?.(entry.id)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
  {:else if tool === "crop"}
    <select
      aria-label={t("options.crop.preset")}
      value={cropPreset(crop)}
      onchange={(e) => chooseCropPreset(e.currentTarget.value)}
    >
      <option value="free">{t("options.crop.free")}</option>
      <option value="original">{t("options.crop.original")}</option>
      {#each CROP_RATIOS as [w, h] (`${w}:${h}`)}
        <option value="{w}:{h}">{w} : {h}</option>
      {/each}
      <option value="ratio">{t("options.crop.ratio")}</option>
      <option value="size">{t("options.crop.size")}</option>
    </select>
    <button
      class="icon-btn"
      class:on={straighten}
      aria-pressed={straighten}
      onmousedown={keepFocus}
      title={t("options.crop.straightenHint")}
      aria-label={t("options.crop.straighten")}
      onclick={() => (straighten = !straighten)}
    >
      <Icon name="straighten" />
    </button>
    {#if crop.mode !== "free"}
      {@const size = crop.mode === "size"}
      <SliderField
        label={t("options.crop.width")}
        bind:value={crop.width}
        min={size ? 1 : 0.01}
        max={size ? 300000 : 1000}
        step={size ? 1 : 0.01}
        unit={size ? "px" : undefined}
        log
      />
      <button
        class="icon-btn"
        onmousedown={keepFocus}
        title={t("options.crop.swap")}
        aria-label={t("options.crop.swap")}
        onclick={() => {
          if (crop.mode !== "free") crop = { ...crop, width: crop.height, height: crop.width };
        }}
      >
        <Icon name="swap" />
      </button>
      <SliderField
        label={t("options.crop.height")}
        bind:value={crop.height}
        min={size ? 1 : 0.01}
        max={size ? 300000 : 1000}
        step={size ? 1 : 0.01}
        unit={size ? "px" : undefined}
        log
      />
    {/if}
  {:else if tool === "gradient"}
    <select aria-label={t("options.gradient.preset")} bind:value={gradient.preset}>
      <option value="foregroundToBackground">
        {t("options.gradient.foregroundToBackground")}
      </option>
      <option value="foregroundToTransparent">
        {t("options.gradient.foregroundToTransparent")}
      </option>
      <option value="blackToWhite">{t("options.gradient.blackToWhite")}</option>
    </select>
    {#each [{ shape: "linear", icon: "gradientLinear" }, { shape: "radial", icon: "gradientRadial" }] as const as entry (entry.shape)}
      <button
        class="icon-btn"
        class:on={gradient.shape === entry.shape}
        aria-pressed={gradient.shape === entry.shape}
        onmousedown={keepFocus}
        title={t(`options.gradient.${entry.shape}`)}
        aria-label={t(`options.gradient.${entry.shape}`)}
        onclick={() => (gradient.shape = entry.shape)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
    <span class="divider"></span>
    <SliderField
      label={t("options.opacity")}
      bind:value={gradient.opacity}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
    <label class="option">
      <input type="checkbox" bind:checked={gradient.reverse} />
      {t("options.gradient.reverse")}
    </label>
  {:else if shapeKindOf(tool) || tool === "pen"}
    <!-- The Pen draws a shape of a path, filled and stroked as the shape tools' (ADR 0041). -->
    {@const kind = shapeKindOf(tool) ?? "path"}
    {#if kind !== "line"}
      <label class="option">
        <input type="checkbox" bind:checked={shape.filled} />
        {t("options.shape.fill")}
      </label>
      <button
        class="swatch"
        style:background={shape.fill}
        class:off={!shape.filled}
        onmousedown={keepFocus}
        title={t("options.shape.fillColor")}
        aria-label={t("options.shape.fillColor")}
        onclick={() => onpickcolor?.("fill")}
      ></button>
      <span class="divider"></span>
      <label class="option">
        <input type="checkbox" bind:checked={shape.stroked} />
        {t("options.shape.stroke")}
      </label>
    {/if}
    <button
      class="swatch"
      style:background={shape.stroke}
      class:off={kind !== "line" && !shape.stroked}
      onmousedown={keepFocus}
      title={t("options.shape.strokeColor")}
      aria-label={t("options.shape.strokeColor")}
      onclick={() => onpickcolor?.("stroke")}
    ></button>
    <SliderField
      label={t("options.shape.width")}
      bind:value={shape.strokeWidth}
      min={0.1}
      max={MAX_STROKE_WIDTH}
      step={0.1}
      unit="px"
      log
    />
    {#if kind !== "line"}
      <select aria-label={t("options.shape.align")} bind:value={shape.strokeAlign}>
        {#each ["inside", "center", "outside"] as const as align (align)}
          <option value={align}>{t(`options.shape.align.${align}`)}</option>
        {/each}
      </select>
    {/if}
    {#if kind === "rectangle"}
      <span class="divider"></span>
      <SliderField
        label={t("options.shape.radius")}
        bind:value={shape.radius}
        min={0}
        max={MAX_RADIUS}
        unit="px"
      />
    {:else if kind === "polygon"}
      <span class="divider"></span>
      <SliderField
        label={t("options.shape.sides")}
        bind:value={shape.sides}
        min={MIN_SIDES}
        max={MAX_SIDES}
        width={40}
      />
      <label class="option">
        <input type="checkbox" bind:checked={shape.star} />
        {t("options.shape.star")}
      </label>
      {#if shape.star}
        <SliderField
          label={t("options.shape.starRatio")}
          bind:value={shape.starRatio}
          min={1}
          max={100}
          unit="%"
          factor={100}
          width={44}
        />
      {/if}
    {/if}
  {:else if tool === "paintBucket"}
    <SliderField
      label={t("options.opacity")}
      bind:value={bucket.opacity}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
    <span class="divider"></span>
    <SliderField label={t("options.tolerance")} bind:value={bucket.tolerance} min={0} max={255} />
    <label class="option">
      <input type="checkbox" bind:checked={bucket.antiAlias} />
      {t("options.antiAlias")}
    </label>
    <label class="option">
      <input type="checkbox" bind:checked={bucket.contiguous} />
      {t("options.contiguous")}
    </label>
    <label class="option">
      <input type="checkbox" bind:checked={bucket.sampleAll} />
      {t("options.sampleAll")}
    </label>
  {:else if tool === "eyedropper"}
    <label class="option">
      {t("options.sampleSize")}
      <select bind:value={eyedropper.size}>
        {#each SAMPLE_SIZES as size (size)}
          <option value={size}>
            {size === 1 ? t("options.sampleSize.point") : t("options.sampleSize.average", { size })}
          </option>
        {/each}
      </select>
    </label>
    <label class="option">
      {t("options.sample")}
      <select bind:value={eyedropper.sample}>
        <option value="all">{t("options.sample.all")}</option>
        <option value="layer">{t("options.sample.layer")}</option>
      </select>
    </label>
  {:else if tool === "healingBrush" && heal.mode === "patch"}
    <label class="option">
      {t("options.heal.mode")}
      <select bind:value={heal.mode}>
        <option value="brush">{t("options.heal.brush")}</option>
        <option value="patch">{t("options.heal.patch")}</option>
      </select>
    </label>
    <label class="option">
      {t("options.sample")}
      <select bind:value={heal.sample}>
        <option value="layer">{t("options.sample.layer")}</option>
        <option value="all">{t("options.sample.all")}</option>
      </select>
    </label>
    <span class="hint">{t("patch.hint")}</span>
  {:else if tool === "blur" || tool === "sharpen" || tool === "smudge"}
    <BrushPicker bind:size={focus.size} bind:hardness={focus.hardness} />
    <span class="divider"></span>
    <SliderField
      label={t("options.focus.strength")}
      bind:value={focus.strength}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
  {:else if tool === "dodge" || tool === "burn"}
    <BrushPicker bind:size={tone.size} bind:hardness={tone.hardness} />
    <span class="divider"></span>
    <label class="option">
      {t("options.tone.range")}
      <select bind:value={tone.range}>
        <option value="shadows">{t("options.tone.shadows")}</option>
        <option value="midtones">{t("options.tone.midtones")}</option>
        <option value="highlights">{t("options.tone.highlights")}</option>
      </select>
    </label>
    <SliderField
      label={t("options.tone.exposure")}
      bind:value={tone.exposure}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
  {:else if paint}
    {#if tool === "healingBrush"}
      <label class="option">
        {t("options.heal.mode")}
        <select bind:value={heal.mode}>
          <option value="brush">{t("options.heal.brush")}</option>
          <option value="patch">{t("options.heal.patch")}</option>
        </select>
      </label>
      <span class="divider"></span>
    {/if}
    <BrushPicker bind:size={paint.size} bind:hardness={paint.hardness} />
    <span class="divider"></span>
    <SliderField
      label={t("options.opacity")}
      bind:value={paint.opacity}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
    <SliderField
      label={t("options.flow")}
      bind:value={paint.flow}
      min={0}
      max={100}
      unit="%"
      factor={100}
      width={44}
    />
    <span class="divider"></span>
    <label class="option" title={t("options.pressureSize.hint")}>
      <input type="checkbox" bind:checked={paint.pressureSize} />
      {t("options.pressureSize")}
    </label>
    <label class="option" title={t("options.pressureOpacity.hint")}>
      <input type="checkbox" bind:checked={paint.pressureOpacity} />
      {t("options.pressureOpacity")}
    </label>
    {#if tool === "brush" || tool === "eraser"}
      <label class="option" title={t("options.pencil.hint")}>
        <input type="checkbox" bind:checked={paint.pencil} />
        {t("options.pencil")}
      </label>
    {/if}
    {#if tool === "cloneStamp" || tool === "healingBrush"}
      {@const source = tool === "cloneStamp" ? clone : heal}
      <span class="divider"></span>
      <label class="option" title={t("options.clone.aligned.hint")}>
        <input type="checkbox" bind:checked={source.aligned} />
        {t("options.clone.aligned")}
      </label>
      <label class="option">
        {t("options.sample")}
        <select bind:value={source.sample}>
          <option value="layer">{t("options.sample.layer")}</option>
          <option value="all">{t("options.sample.all")}</option>
        </select>
      </label>
    {/if}
  {:else if isSelectionTool(tool)}
    <!-- Quick Selection has no intersection, as in Photoshop. -->
    {#each MODES.filter((m) => tool !== "quickSelection" || m.mode !== "intersect") as entry (entry.mode)}
      <button
        class="icon-btn"
        class:on={selectionMode === entry.mode}
        onmousedown={keepFocus}
        aria-pressed={selectionMode === entry.mode}
        title={t(entry.label)}
        aria-label={t(entry.label)}
        onclick={() => (selectionMode = entry.mode)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
    <span class="divider"></span>
    {#if tool === "quickSelection"}
      <SliderField
        label={t("options.brushSize")}
        bind:value={quick.size}
        min={1}
        max={MAX_BRUSH}
        unit="px"
        log
      />
      <label class="option">
        <input type="checkbox" bind:checked={quick.sampleAll} />
        {t("options.sampleAll")}
      </label>
    {:else if tool === "objectSelection"}
      <label class="option">
        <input type="checkbox" bind:checked={quick.sampleAll} />
        {t("options.sampleAll")}
      </label>
      <label class="option" title={t("options.refineEdge.hint")}>
        <input type="checkbox" bind:checked={quick.objectRefine} />
        {t("options.refineEdge")}
      </label>
    {:else if tool === "wand"}
      <SliderField label={t("options.tolerance")} bind:value={wand.tolerance} min={0} max={255} />
      <label class="option">
        <input type="checkbox" bind:checked={antiAlias} />
        {t("options.antiAlias")}
      </label>
      <label class="option">
        <input type="checkbox" bind:checked={wand.contiguous} />
        {t("options.contiguous")}
      </label>
      <label class="option">
        <input type="checkbox" bind:checked={wand.sampleAll} />
        {t("options.sampleAll")}
      </label>
    {:else}
      <SliderField
        label={t("options.feather")}
        bind:value={feather}
        min={0}
        max={MAX_FEATHER}
        unit="px"
      />
      {#if tool !== "marquee"}
        <label class="option">
          <input type="checkbox" bind:checked={antiAlias} />
          {t("options.antiAlias")}
        </label>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .options {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 0 8px;
    background: var(--chrome);
    border-bottom: 1px solid var(--border-dark);
  }

  .tool-icon {
    display: grid;
    place-items: center;
    width: 24px;
    color: var(--text);
  }

  .divider {
    align-self: stretch;
    width: 1px;
    margin: 6px 2px;
    background: var(--border-strong);
  }

  .option {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .hint {
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* A shape's fill or stroke color; faded while the shape has none. */
  .swatch {
    width: 22px;
    height: 18px;
    padding: 0;
    border: 1px solid var(--border, #555);
    border-radius: 3px;
    cursor: pointer;
  }

  .swatch.off {
    opacity: 0.35;
  }

  .option input[type="checkbox"] {
    margin: 0;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }

  /* Quick Mask's label: the overlay's red, so that the mode reads at a glance. */
  .mode {
    padding: 1px 8px;
    border-radius: 9px;
    background: #c0392b;
    color: #ffffff;
    font-weight: 600;
    white-space: nowrap;
  }
</style>

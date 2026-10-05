<script lang="ts">
  // The options bar (ADR 0013): the settings of the active tool, under the menu bar. Only what
  // belongs to the tool: view and apply/cancel commands live in the menus and on the keys.
  import type { SelectionMode } from "./engine";
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
    eyedropper = $bindable({ sample: "all", size: 1 }),
    crop = $bindable({ mode: "free" }),
    canvasSize = { width: 1, height: 1 },
    transform,
    quickMask = false,
    quickMaskOpacity = $bindable(50),
    alignable = false,
    distributable = false,
    onalign,
    ondistribute,
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
    /** Eyedropper: every visible layer or the active one alone, and the side of the average. */
    eyedropper?: { sample: "all" | "layer"; size: number };
    /** Crop: the frame's ratio or size (pixels), if any. */
    crop?: CropAspect;
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
  };

  /** The painting tool's options, edited in place. */
  const paint = $derived(
    tool === "eraser" || tool === "restoreEraser" ? eraser : tool === "brush" ? brush : null,
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
  {:else if paint}
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

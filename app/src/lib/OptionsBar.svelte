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
    transform,
  }: {
    tool: ToolId;
    /** Free Transform under way: its fields replace the tool's options, as in Photoshop. */
    transform?: Snippet;
    /** Move tool: a drag takes the layer under the pointer (Ctrl inverts it). */
    autoSelect: boolean;
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

  {#if transform}
    {@render transform()}
  {:else if tool === "move"}
    <label class="option">
      <input type="checkbox" bind:checked={autoSelect} />
      {t("options.autoSelect")}
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
</style>

<script lang="ts" module>
  /** A brush preset: a diameter in document pixels and a hardness in [0, 1]. */
  export type BrushPreset = { size: number; hardness: number };

  /** Photoshop's round brushes (General Brushes): hard and soft, at common sizes. */
  const BUILT_IN: BrushPreset[] = [1, 3, 5, 9, 13, 19, 30, 60, 100, 200, 300, 500].flatMap(
    (size) => [
      { size, hardness: 1 },
      { size, hardness: 0 },
    ],
  );

  const STORAGE_KEY = "slopshop.brushPresets";

  /** The presets the user saved (a per-machine convenience; absent storage: none). */
  function loadSaved(): BrushPreset[] {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      if (!Array.isArray(parsed)) return [];
      return parsed.filter(
        (p): p is BrushPreset =>
          typeof p?.size === "number" &&
          typeof p?.hardness === "number" &&
          p.size >= 1 &&
          p.hardness >= 0 &&
          p.hardness <= 1,
      );
    } catch {
      return [];
    }
  }

  function storeSaved(presets: BrushPreset[]) {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(presets));
    } catch {
      // Storage unavailable: the presets last for the session.
    }
  }
</script>

<script lang="ts">
  // The brush preset picker (ADR 0027), as Photoshop's: the tip shown in the options bar drops
  // down a panel with the size and hardness and a grid of round brushes, hard and soft, plus the
  // user's own (+ saves the current brush; a right-click removes a saved one). Picking a preset
  // sets the size and hardness; opacity, flow and pressure stay as they are.
  import SliderField from "./SliderField.svelte";
  import { t } from "./i18n/index.svelte";
  import { MAX_BRUSH } from "./selection";
  import { keepFocus } from "./platform";

  let {
    size = $bindable(),
    hardness = $bindable(),
  }: {
    /** Diameter, document pixels. */
    size: number;
    /** In [0, 1]. */
    hardness: number;
  } = $props();

  let open = $state(false);
  let saved = $state<BrushPreset[]>(loadSaved());
  let root: HTMLElement;

  /** A tip drawn as Photoshop draws it: full within the hardness, fading to the edge. */
  function tip(preset: BrushPreset): string {
    const inner = Math.round(preset.hardness * 100);
    return `radial-gradient(circle, var(--text) ${Math.min(inner, 99)}%, transparent 100%)`;
  }

  /** On screen, sizes are capped: a 500 px brush is drawn as a large dot. */
  function dot(preset: BrushPreset, largest: number): number {
    return Math.max(2, Math.min(largest, Math.sqrt(preset.size) * 3));
  }

  function pick(preset: BrushPreset) {
    size = preset.size;
    hardness = preset.hardness;
  }

  function save() {
    const preset = { size, hardness };
    if (saved.some((p) => p.size === size && p.hardness === hardness)) return;
    saved = [...saved, preset];
    storeSaved(saved);
  }

  function remove(index: number) {
    saved = saved.filter((_, i) => i !== index);
    storeSaved(saved);
  }

  const current = $derived({ size, hardness });
</script>

<svelte:window
  onpointerdown={(e) => {
    if (open && !root.contains(e.target as Node)) open = false;
  }}
  onkeydown={(e) => {
    if (open && e.key === "Escape") {
      e.stopPropagation();
      open = false;
    }
  }}
/>

<span class="picker" bind:this={root}>
  <button
    class="current"
    class:on={open}
    title={t("brush.picker")}
    aria-label={t("brush.picker")}
    aria-expanded={open}
    onmousedown={keepFocus}
    onclick={() => (open = !open)}
  >
    <span class="tip" style:background={tip(current)} style:width="18px" style:height="18px"></span>
    <span class="size">{size}</span>
    <span class="arrow">▾</span>
  </button>

  {#if open}
    <div class="panel" role="dialog" aria-label={t("brush.picker")}>
      <div class="sliders">
        <SliderField
          label={t("options.brushSize")}
          bind:value={size}
          min={1}
          max={MAX_BRUSH}
          unit="px"
          log
        />
        <SliderField
          label={t("options.hardness")}
          bind:value={hardness}
          min={0}
          max={100}
          unit="%"
          factor={100}
        />
      </div>
      <div class="grid">
        {#each BUILT_IN as preset, index (index)}
          <button
            class="preset"
            class:on={preset.size === size && preset.hardness === hardness}
            title={t(preset.hardness === 1 ? "brush.hardRound" : "brush.softRound", {
              size: preset.size,
            })}
            onmousedown={keepFocus}
            onclick={() => pick(preset)}
          >
            <span
              class="tip"
              style:background={tip(preset)}
              style:width="{dot(preset, 28)}px"
              style:height="{dot(preset, 28)}px"
            ></span>
            <span class="label">{preset.size}</span>
          </button>
        {/each}
        {#each saved as preset, index (index)}
          <button
            class="preset saved"
            class:on={preset.size === size && preset.hardness === hardness}
            title={t("brush.saved", {
              size: preset.size,
              hardness: Math.round(preset.hardness * 100),
            })}
            onmousedown={keepFocus}
            onclick={() => pick(preset)}
            oncontextmenu={(e) => {
              e.preventDefault();
              remove(index);
            }}
          >
            <span
              class="tip"
              style:background={tip(preset)}
              style:width="{dot(preset, 28)}px"
              style:height="{dot(preset, 28)}px"
            ></span>
            <span class="label">{preset.size}</span>
          </button>
        {/each}
        <button
          class="preset add"
          title={t("brush.save")}
          aria-label={t("brush.save")}
          onmousedown={keepFocus}
          onclick={save}
        >
          +
        </button>
      </div>
    </div>
  {/if}
</span>

<style>
  .picker {
    position: relative;
    display: inline-flex;
  }

  .current {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 4px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: var(--text);
  }

  .current:hover,
  .current.on {
    border-color: var(--border-strong);
    background: var(--overlay-hover);
  }

  .size {
    min-width: 22px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .arrow {
    color: var(--text-muted);
    font-size: 10px;
  }

  .tip {
    display: inline-block;
    flex: none;
    border-radius: 50%;
  }

  .panel {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 30;
    display: grid;
    gap: 8px;
    width: 300px;
    padding: 10px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 6px 20px #0008;
  }

  .sliders {
    display: grid;
    gap: 6px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 4px;
    max-height: 220px;
    overflow-y: auto;
  }

  .preset {
    display: grid;
    grid-template-rows: 30px auto;
    place-items: center;
    padding: 4px 0 2px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: var(--chrome);
    color: var(--text-muted);
    font-size: 11px;
  }

  .preset:hover {
    border-color: var(--border-strong);
  }

  .preset.on {
    border-color: var(--accent);
    color: var(--text);
  }

  .preset.saved {
    background: var(--overlay-hover);
  }

  .preset.add {
    grid-template-rows: none;
    font-size: 18px;
  }
</style>

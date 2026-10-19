<script lang="ts">
  // A layer style's Contour picker (ADR 0032), as Photoshop's: the current contour's curve, a
  // click drops down Photoshop's contours (Linear, Cone, Gaussian…), each drawn; picking one sets
  // it. A contour that is none of them (from a Photoshop document) shows as Custom. The curves
  // come from the engine (`contour_presets`, the contour's own samples).
  import { onMount } from "svelte";
  import type { ContourPreset, StyleContour } from "./engine";
  import {
    contourPath,
    contourPresets,
    presetOf,
    samplesOf,
    type ContourPresetId,
  } from "./styleContours";
  import { t } from "./i18n/index.svelte";

  let {
    contour,
    absent,
    label,
    onpick,
  }: {
    /** The effect's contour; absent, its default. */
    contour: StyleContour | undefined;
    /** What an absent contour is (Linear; Satin's Gaussian). */
    absent: ContourPresetId;
    /** What the picker sets (Contour, Gloss Contour). */
    label: string;
    onpick: (contour: StyleContour) => void;
  } = $props();

  const THUMB = 28;

  let presets = $state<ContourPreset[]>([]);
  let open = $state(false);
  let root: HTMLElement;

  onMount(() => {
    let current = true;
    void contourPresets().then((list) => {
      if (current) presets = list;
    });
    return () => {
      current = false;
    };
  });

  const selected = $derived(presetOf(contour, absent, presets));
  const name = $derived(t(selected ? `style.contour.${selected}` : "style.contour.custom"));
  const path = $derived.by(() => {
    const samples = samplesOf(contour, absent, presets);
    return samples ? contourPath(samples, THUMB) : "";
  });

  function pick(preset: ContourPreset) {
    open = false;
    onpick(structuredClone($state.snapshot(preset.contour)) as StyleContour);
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (open && !root.contains(e.target as Node)) open = false;
  }}
/>

<div class="field">
  <span>{label}</span>
  <span class="picker" bind:this={root}>
    <button
      type="button"
      class="current"
      class:on={open}
      title={name}
      aria-label={label}
      aria-expanded={open}
      onclick={() => (open = !open)}
    >
      <svg width={THUMB} height={THUMB} viewBox="0 0 {THUMB} {THUMB}" aria-hidden="true">
        <path d={path} />
      </svg>
      <span class="arrow">▾</span>
    </button>
    {#if open}
      <div class="panel" role="listbox" aria-label={label}>
        {#each presets as preset (preset.id)}
          <button
            type="button"
            role="option"
            class="preset"
            aria-selected={preset.id === selected}
            class:on={preset.id === selected}
            title={t(`style.contour.${preset.id as ContourPresetId}`)}
            aria-label={t(`style.contour.${preset.id as ContourPresetId}`)}
            onclick={() => pick(preset)}
          >
            <svg width={THUMB} height={THUMB} viewBox="0 0 {THUMB} {THUMB}" aria-hidden="true">
              <path d={contourPath(preset.contour.samples ?? [], THUMB)} />
            </svg>
          </button>
        {/each}
      </div>
    {/if}
  </span>
</div>

<style>
  .field {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .field > span:first-child {
    min-width: 90px;
  }

  .picker {
    position: relative;
    display: inline-flex;
  }

  .current {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px;
    border: 1px solid var(--border-strong);
    border-radius: 3px;
    background: var(--chrome);
    color: var(--text);
  }

  .current.on,
  .current:hover {
    background: var(--overlay-hover);
  }

  .arrow {
    color: var(--text-muted);
    font-size: 10px;
  }

  svg {
    display: block;
    background: var(--panel-header);
  }

  path {
    fill: none;
    stroke: var(--text);
    stroke-width: 1.5;
  }

  .panel {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 30;
    display: grid;
    grid-template-columns: repeat(6, auto);
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 6px 20px #0008;
  }

  .preset {
    padding: 2px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: var(--chrome);
  }

  .preset:hover {
    border-color: var(--border-strong);
  }

  .preset.on {
    border-color: var(--accent);
  }
</style>

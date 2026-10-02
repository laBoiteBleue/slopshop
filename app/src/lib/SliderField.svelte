<script lang="ts">
  // A number setting as in Photoshop's options bar: a field, a ▾ that drops a slider down under
  // it, and a label that changes the value when dragged sideways (a "scrubby" label). Sizes
  // that span several orders of magnitude (brush sizes) use a logarithmic slider, so that small
  // sizes stay as easy to set as large ones. The value changes live while sliding.
  import { keepFocus } from "./platform";

  let {
    label,
    value = $bindable(),
    min,
    max,
    step = 1,
    unit = "",
    log = false,
    title,
    width = 52,
    factor = 1,
  }: {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    /** Shown after the field (`px`, `%`). */
    unit?: string;
    /** A logarithmic slider (min must be above 0). */
    log?: boolean;
    title?: string;
    /** Width of the field, CSS pixels. */
    width?: number;
    /** The value is shown multiplied by this (100: a share in [0, 1] shown in %); `min`,
     * `max` and `step` are in shown units. */
    factor?: number;
  } = $props();

  /** The value in shown units. */
  const shown = $derived(Number((value * factor).toFixed(6)));

  /** Slider positions, for a logarithmic slider. */
  const STEPS = 1000;
  let open = $state(false);
  let root: HTMLElement;

  function clamp(v: number): number {
    const snapped = Math.round(v / step) * step;
    // Floating steps (0.1) must not leave 0.30000000000000004 behind.
    const rounded = Number(snapped.toFixed(6));
    return Math.min(Math.max(rounded, min), max);
  }

  /** Set from shown units. */
  function set(v: number) {
    if (Number.isFinite(v)) value = clamp(v) / factor;
  }

  const position = $derived(log ? (Math.log(shown / min) / Math.log(max / min)) * STEPS : shown);

  function fromPosition(p: number): number {
    return log ? min * Math.pow(max / min, p / STEPS) : p;
  }

  // Scrubby label: each pixel of drag moves the value by one step (a percent of the value for
  // a logarithmic setting), Shift ten times faster.
  let scrub: { x: number; value: number } | null = null;

  function scrubStart(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    scrub = { x: e.clientX, value: shown };
  }

  function scrubMove(e: PointerEvent) {
    if (!scrub) return;
    const dx = (e.clientX - scrub.x) * (e.shiftKey ? 10 : 1);
    set(log ? scrub.value * Math.pow(1.01, dx) : scrub.value + dx * step);
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (open && !root.contains(e.target as Node)) open = false;
  }}
  onkeydown={(e) => {
    if (open && (e.key === "Escape" || e.key === "Enter")) {
      e.stopPropagation();
      open = false;
    }
  }}
/>

<span class="field" bind:this={root} {title}>
  <span
    class="label"
    role="slider"
    tabindex="-1"
    aria-valuenow={shown}
    aria-valuemin={min}
    aria-valuemax={max}
    onpointerdown={scrubStart}
    onpointermove={scrubMove}
    onpointerup={() => (scrub = null)}
    onpointercancel={() => (scrub = null)}
  >
    {label}
  </span>
  <input
    type="number"
    {min}
    {max}
    {step}
    style:width="{width}px"
    value={shown}
    oninput={(e) => set(e.currentTarget.valueAsNumber)}
    onchange={(e) => (e.currentTarget.valueAsNumber = shown)}
  />
  {#if unit}<span class="unit">{unit}</span>{/if}
  <button
    class="arrow"
    class:on={open}
    tabindex="-1"
    aria-label={label}
    onmousedown={keepFocus}
    onclick={() => (open = !open)}
  >
    ▾
  </button>
  {#if open}
    <span class="popup">
      <input
        type="range"
        min={log ? 0 : min}
        max={log ? STEPS : max}
        step={log ? 1 : step}
        value={position}
        oninput={(e) => set(fromPosition(e.currentTarget.valueAsNumber))}
      />
    </span>
  {/if}
</span>

<style>
  .field {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  /* Drag sideways to change the value, as Photoshop's labels. */
  .label {
    cursor: ew-resize;
    user-select: none;
  }

  .unit {
    color: var(--text-muted);
  }

  .arrow {
    width: 14px;
    height: 20px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 10px;
    line-height: 1;
  }

  .arrow:hover,
  .arrow.on {
    color: var(--text);
  }

  .popup {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    display: flex;
    padding: 8px 10px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 6px 20px #0008;
  }

  .popup input[type="range"] {
    width: 160px;
    accent-color: var(--accent);
  }
</style>

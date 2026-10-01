<script lang="ts">
  // The Magic Wand (W, ADR 0024): a click selects the pixels of a color similar to the clicked
  // one (the options bar's tolerance), connected to it or not. As with the other selection tools,
  // Shift, Alt or both at the click add, subtract or intersect, shown by a badge by the pointer.
  import type { SelectionMode } from "./engine";
  import { MODE_BADGES, modeFromKeys } from "./selection";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    mode,
    onpick,
  }: {
    mapping: ViewMapping;
    /** The options bar's mode, which keys override. */
    mode: SelectionMode;
    /** A click on document pixel (`x`, `y`); `mode`: the one the keys asked for, or null. */
    onpick: (x: number, y: number, mode: SelectionMode | null) => void;
  } = $props();

  let hover = $state<{ x: number; y: number } | null>(null);
  let keys = $state({ shiftKey: false, altKey: false });
  let element: SVGSVGElement;
  const badge = $derived(MODE_BADGES[modeFromKeys(keys) ?? mode]);

  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }

  function pick(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    if (x < 0 || y < 0) return;
    onpick(Math.floor(x), Math.floor(y), modeFromKeys(e));
  }
</script>

<svelte:window onkeydown={track} onkeyup={track} />

<svg
  class="wand"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={pick}
  onpointermove={(e) => {
    track(e);
    const box = element.getBoundingClientRect();
    hover = { x: e.clientX - box.left, y: e.clientY - box.top };
  }}
  onpointerleave={() => (hover = null)}
>
  {#if hover && badge && !mapping.hand}
    <text class="badge" x={hover.x + 8} y={hover.y + 16}>{badge}</text>
  {/if}
</svg>

<style>
  .wand {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .wand.hand {
    pointer-events: none;
  }

  .badge {
    fill: #ffffff;
    font-size: 13px;
    font-weight: 700;
    paint-order: stroke;
    stroke: #000000;
    stroke-width: 3px;
    pointer-events: none;
  }
</style>

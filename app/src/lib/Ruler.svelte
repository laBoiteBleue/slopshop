<script lang="ts">
  // A ruler along the image (View > Rulers): the document's coordinates in pixels, labelled on
  // the major ticks, as in Photoshop. A press on it drags out a guide (the viewport follows it).
  import { rulerTicks } from "./rulers";

  let {
    vertical,
    start,
    docPerCss,
    length,
    onpress,
  }: {
    /** The left ruler (y coordinates, top to bottom); else the top one (x, left to right). */
    vertical: boolean;
    /** The document coordinate at the ruler's start. */
    start: number;
    docPerCss: number;
    /** CSS pixels. */
    length: number;
    /** A press on the ruler: a guide is dragged out of it. */
    onpress: (e: PointerEvent) => void;
  } = $props();

  /** The ruler's breadth, CSS pixels (as the frame's grid lays it out). */
  const BREADTH = 18;
  const MINOR = 4;

  const ticks = $derived(rulerTicks(start, docPerCss, length));
</script>

<svg
  class="ruler"
  class:vertical
  width={vertical ? BREADTH : length}
  height={vertical ? length : BREADTH}
  role="presentation"
  onpointerdown={onpress}
>
  {#each ticks as tick (tick.value)}
    {@const at = Math.round(tick.at) + 0.5}
    {@const depth = tick.major ? BREADTH : MINOR}
    {#if vertical}
      <line x1={BREADTH - depth} x2={BREADTH} y1={at} y2={at} />
      {#if tick.major}
        <text
          x={BREADTH - 5}
          y={at + 3}
          text-anchor="end"
          transform="rotate(-90 {BREADTH - 5} {at + 3})"
        >
          {tick.value}
        </text>
      {/if}
    {:else}
      <line x1={at} x2={at} y1={BREADTH - depth} y2={BREADTH} />
      {#if tick.major}<text x={at + 3} y={10}>{tick.value}</text>{/if}
    {/if}
  {/each}
</svg>

<style>
  .ruler {
    display: block;
    /* Opaque: with native presentation, nothing else draws under the rulers. */
    background: var(--chrome);
    cursor: default;
    touch-action: none;
  }

  .ruler:not(.vertical) {
    border-bottom: 1px solid var(--border-dark);
  }

  .ruler.vertical {
    border-right: 1px solid var(--border-dark);
  }

  line {
    stroke: var(--text-muted);
    stroke-width: 1;
  }

  text {
    fill: var(--text-muted);
    font-size: 9px;
    user-select: none;
  }
</style>

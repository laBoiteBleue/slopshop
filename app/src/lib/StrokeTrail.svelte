<script lang="ts">
  // The path of a stroke under way, as wide as the brush (Select and Mask's refine-edge brush:
  // what it paints shows once the stroke is released and the edge is detected again).
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    points,
    size,
  }: {
    mapping: ViewMapping;
    /** Document pixels. */
    points: [number, number][];
    /** Brush diameter, document pixels. */
    size: number;
  } = $props();

  const path = $derived(
    points
      .map(([x, y], i) => {
        const [vx, vy] = mapping.toViewport(x, y);
        return `${i === 0 ? "M" : "L"}${vx} ${vy}`;
      })
      .join(""),
  );
</script>

<svg class="trail" aria-hidden="true">
  {#if points.length > 0}
    <path d={path} stroke-width={size / mapping.docPerCss} />
  {/if}
</svg>

<style>
  .trail {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  path {
    fill: none;
    stroke: var(--accent);
    stroke-opacity: 0.45;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>

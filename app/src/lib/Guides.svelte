<svelte:options namespace="svg" />

<script lang="ts">
  // Smart guides of a snap, magenta as in Photoshop, drawn inside an overlay's <svg>. Measures
  // (equal sizes) end with short ticks, like the serifs of an I, a fixed size on screen.
  import type { Guide } from "./snap";
  import type { ViewMapping } from "./Viewport.svelte";

  let { guides, mapping }: { guides: Guide[]; mapping: ViewMapping } = $props();

  /** Half the length of a measure's end ticks, in CSS pixels. */
  const SERIF_PX = 4;
</script>

{#each guides as guide, i (i)}
  {@const [x1, y1] = mapping.toViewport(guide.x1, guide.y1)}
  {@const [x2, y2] = mapping.toViewport(guide.x2, guide.y2)}
  <line class="guide" {x1} {y1} {x2} {y2} />
  {#if guide.measure}
    {@const length = Math.hypot(x2 - x1, y2 - y1) || 1}
    {@const nx = ((y1 - y2) / length) * SERIF_PX}
    {@const ny = ((x2 - x1) / length) * SERIF_PX}
    <line class="guide" x1={x1 - nx} y1={y1 - ny} x2={x1 + nx} y2={y1 + ny} />
    <line class="guide" x1={x2 - nx} y1={y2 - ny} x2={x2 + nx} y2={y2 + ny} />
  {/if}
{/each}

<style>
  .guide {
    stroke: #ff2bd6;
    stroke-width: 1;
    pointer-events: none;
  }
</style>

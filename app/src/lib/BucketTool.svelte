<script lang="ts">
  // The Paint Bucket (G): a click on the image fills the region of a similar color there (the
  // owner asks the engine). The pointer is a crosshair; Space lets the pan through.
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    onpick,
  }: {
    mapping: ViewMapping;
    /** A click on document pixel (`x`, `y`), possibly outside the canvas. */
    onpick: (x: number, y: number) => void;
  } = $props();
</script>

<div
  class="bucket"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={(e) => {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    onpick(Math.floor(x), Math.floor(y));
  }}
></div>

<style>
  .bucket {
    position: absolute;
    inset: 0;
    cursor: crosshair;
  }

  .bucket.hand {
    pointer-events: none;
  }
</style>

<script lang="ts" module>
  /** Thumbnails already received, by pattern and size: a library pattern never changes. */
  const cache = new Map<string, ImageData>();
</script>

<script lang="ts">
  // A library pattern's thumbnail (ADR 0042), fitted in a square box.
  import { engine } from "./engine";

  let { pattern, size }: { pattern: string; size: number } = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);

  function draw(image: ImageData) {
    if (!canvas) return;
    canvas.width = image.width;
    canvas.height = image.height;
    canvas.getContext("2d")?.putImageData(image, 0, 0);
  }

  $effect(() => {
    if (!canvas) return;
    const maxSide = Math.round(size * window.devicePixelRatio);
    const key = `${pattern}:${maxSide}`;
    const known = cache.get(key);
    if (known) {
      draw(known);
      return;
    }
    let current = true;
    engine
      .patternThumbnail(pattern, maxSide)
      .then((image) => {
        cache.set(key, image);
        if (current) draw(image);
      })
      .catch(() => {
        // The pattern's file went away: nothing to show.
      });
    return () => {
      current = false;
    };
  });
</script>

<span class="box" style:width="{size}px" style:height="{size}px">
  <canvas bind:this={canvas}></canvas>
</span>

<style>
  /* A checkerboard behind transparency, as layer thumbnails. */
  .box {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 50% / 8px 8px;
  }

  canvas {
    max-width: 100%;
    max-height: 100%;
    image-rendering: pixelated;
  }
</style>

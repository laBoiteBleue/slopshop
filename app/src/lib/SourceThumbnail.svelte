<script lang="ts" module>
  /**
   * Thumbnails already received, by source and size: a source never changes (ADR 0040), so its
   * thumbnail is fetched once. Oldest entries go first beyond the limit.
   */
  const cache = new Map<string, ImageData>();
  const CACHE_LIMIT = 256;

  function remember(key: string, image: ImageData) {
    cache.delete(key);
    cache.set(key, image);
    while (cache.size > CACHE_LIMIT) {
      const oldest = cache.keys().next().value;
      if (oldest === undefined) break;
      cache.delete(oldest);
    }
  }
</script>

<script lang="ts">
  // A source's thumbnail (the Sources panel): its own pixels, whatever its layers applied.
  import { engine } from "./engine";

  let {
    documentId,
    source,
    size,
  }: {
    documentId: number;
    source: number;
    /** Side of the square box, in CSS pixels. */
    size: number;
  } = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);
  /** The thumbnail's pixel size, for its aspect ratio in the box. */
  let shape = $state<{ width: number; height: number } | null>(null);

  function draw(image: ImageData) {
    if (!canvas) return;
    canvas.width = image.width;
    canvas.height = image.height;
    canvas.getContext("2d")?.putImageData(image, 0, 0);
    shape = { width: image.width, height: image.height };
  }

  $effect(() => {
    if (!canvas) return;
    // Device pixels, so that the thumbnail stays sharp on high-density screens.
    const maxSide = Math.round(size * window.devicePixelRatio);
    const key = `${source}:${maxSide}`;
    const known = cache.get(key);
    if (known) {
      draw(known);
      return;
    }
    let current = true;
    engine
      .sourceThumbnail(documentId, source, maxSide)
      .then((image) => {
        remember(key, image);
        if (current) draw(image);
      })
      .catch(() => {
        // The source or its document went away meanwhile: nothing to show.
      });
    return () => {
      current = false;
    };
  });

  /** The thumbnail fitted in the box, keeping its aspect ratio. */
  let fitted = $derived.by(() => {
    if (!shape) return { width: size, height: size };
    const scale = size / Math.max(shape.width, shape.height);
    return { width: shape.width * scale, height: shape.height * scale };
  });
</script>

<span class="box" style:width="{size}px" style:height="{size}px">
  <canvas
    bind:this={canvas}
    class="checker"
    style:width="{fitted.width}px"
    style:height="{fitted.height}px"
  ></canvas>
</span>

<style>
  .box {
    display: grid;
    place-items: center;
    flex: none;
  }

  .checker {
    background: repeating-conic-gradient(#c8c8c8 0 25%, #ffffff 0 50%) 0 0 / 8px 8px;
    outline: 1px solid var(--border-strong);
  }
</style>

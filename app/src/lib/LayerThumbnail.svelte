<script lang="ts" module>
  /**
   * Thumbnails already received, by content key and size: a raster shared by several layers
   * or documents (same content key) is fetched once. Oldest entries go first beyond the limit.
   */
  const cache = new Map<string, ImageData>();
  /** Enough for a stack of hundreds of slices (a thumbnail is a few tens of kilobytes). */
  const CACHE_LIMIT = 1024;

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
  import { engine, type LayerView } from "./engine";

  let {
    documentId,
    layer,
    size,
    mask = false,
  }: {
    documentId: number;
    layer: LayerView;
    /** Side of the square box, in CSS pixels. */
    size: number;
    /** The layer's mask instead of the layer. */
    mask?: boolean;
  } = $props();

  /** What is shown: a raster (the layer's, or its mask), or a fill's color. */
  let key = $derived(mask ? (layer.mask?.contentKey ?? null) : layer.contentKey);
  let isImage = $derived(mask || layer.kind === "raster");

  let canvas = $state<HTMLCanvasElement | null>(null);
  /** Whether the row has been scrolled into view: thumbnails are rendered only then. */
  let seen = $state(false);

  $effect(() => {
    if (!canvas || seen) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          seen = true;
          observer.disconnect();
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(canvas);
    return () => observer.disconnect();
  });
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
    if (!isImage || key === null || !canvas || !seen) return;
    // Device pixels, so that the thumbnail stays sharp on high-density screens.
    const maxSide = Math.round(size * window.devicePixelRatio);
    // Masks are shown raw, layers as light: never the same thumbnail for one image.
    const cacheKey = `${mask ? "mask" : "layer"}:${key}:${maxSide}`;
    const known = cache.get(cacheKey);
    if (known) {
      draw(known);
      return;
    }
    let cancelled = false;
    engine
      .layerThumbnail(documentId, layer.id, maxSide, mask)
      .then((image) => {
        remember(cacheKey, image);
        if (!cancelled) draw(image);
      })
      .catch(() => {
        // The layer or its document went away meanwhile: nothing to show.
      });
    return () => {
      cancelled = true;
    };
  });

  function swatch(color: [number, number, number, number]): string {
    const [r, g, b] = color.map((v) => Math.round(Math.min(Math.max(v, 0), 1) * 255));
    return `rgb(${r} ${g} ${b} / ${color[3]})`;
  }

  /** The thumbnail fitted in the box, keeping its aspect ratio. */
  let fitted = $derived.by(() => {
    if (!shape) return { width: size, height: size };
    const scale = size / Math.max(shape.width, shape.height);
    return { width: shape.width * scale, height: shape.height * scale };
  });
</script>

<span class="box" style:width="{size}px" style:height="{size}px">
  {#if isImage}
    <canvas
      bind:this={canvas}
      class="checker"
      style:width="{fitted.width}px"
      style:height="{fitted.height}px"
    ></canvas>
  {:else}
    <span class="checker fill"><span style:background={swatch(layer.swatch)}></span></span>
  {/if}
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

  .fill {
    width: 100%;
    height: 100%;
  }

  .fill span {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>

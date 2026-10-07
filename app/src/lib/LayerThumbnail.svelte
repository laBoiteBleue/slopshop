<script lang="ts" module>
  /**
   * Thumbnails already received, by content key and size: a raster shared by several layers
   * or documents (same content key) is fetched once. Oldest entries go first beyond the limit.
   */
  const cache = new Map<string, ImageData>();
  /**
   * The last thumbnail shown for each layer and mask: a new one replaces it once it has come,
   * never through an empty box (a merged layer's pixels coming after its preview, ADR 0031).
   */
  const shown = new Map<string, ImageData>();
  /** Enough for a stack of hundreds of slices (a thumbnail is a few tens of kilobytes). */
  const CACHE_LIMIT = 1024;
  /**
   * How long new pixels stay before they replace a thumbnail shown: a slider dragged over the
   * layer's stack gives new pixels at each step, each a whole layer for the engine to evaluate.
   */
  const SETTLE_MS = 200;

  function remember(map: Map<string, ImageData>, key: string, image: ImageData) {
    map.delete(key);
    map.set(key, image);
    while (map.size > CACHE_LIMIT) {
      const oldest = map.keys().next().value;
      if (oldest === undefined) break;
      map.delete(oldest);
    }
  }
</script>

<script lang="ts">
  import { engine, type LayerView } from "./engine";
  import { cssFill } from "./gradientFill";
  import { onceSettled } from "./settle";

  let {
    documentId,
    layer,
    size,
    mask = false,
    document = { width: 1, height: 1 },
  }: {
    documentId: number;
    layer: LayerView;
    /** Side of the square box, in CSS pixels. */
    size: number;
    /** The layer's mask instead of the layer. */
    mask?: boolean;
    /** The document's size: where a gradient fill is drawn. */
    document?: { width: number; height: number };
  } = $props();

  /** What is shown: a raster (the layer's, or its mask), or a fill's color or gradient. */
  let key = $derived(mask ? (layer.mask?.contentKey ?? null) : layer.contentKey);
  /** Being baked (ADR 0031): what it will show, rendered small before its pixels come. */
  let baking = $derived(!mask && layer.baking === true);
  // A pattern fill shows its pattern (ADR 0042).
  let isImage = $derived(mask || layer.kind === "raster" || layer.kind === "patternFill" || baking);

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

  /** Which thumbnail of the layer this is, across the rows that show it. */
  let slot = $derived(`${documentId}:${layer.id}:${mask ? "mask" : "layer"}`);

  function draw(image: ImageData) {
    if (!canvas) return;
    canvas.width = image.width;
    canvas.height = image.height;
    canvas.getContext("2d")?.putImageData(image, 0, 0);
    shape = { width: image.width, height: image.height };
    remember(shown, slot, image);
  }

  // A new box shows the layer's last thumbnail until its own comes.
  $effect(() => {
    if (!isImage || !canvas || shape !== null) return;
    const last = shown.get(slot);
    if (last) draw(last);
  });

  type Request = {
    documentId: number;
    layerId: number;
    maxSide: number;
    mask: boolean;
    cacheKey: string | null;
  };
  /** The thumbnail to show now: an answer for another one is kept, not drawn. */
  let wanted: Request | null = null;
  const wants = (request: Request) =>
    wanted === request || (request.cacheKey !== null && wanted?.cacheKey === request.cacheKey);

  const thumbnails = onceSettled(SETTLE_MS, async (request: Request) => {
    try {
      const image = await engine.layerThumbnail(
        request.documentId,
        request.layerId,
        request.maxSide,
        request.mask,
      );
      if (request.cacheKey !== null) remember(cache, request.cacheKey, image);
      if (wants(request)) draw(image);
    } catch {
      // The layer or its document went away meanwhile: nothing to show.
    }
  });

  $effect(() => {
    if (!isImage || key === null || !canvas || !seen) {
      wanted = null;
      thumbnails.drop();
      return;
    }
    // Device pixels, so that the thumbnail stays sharp on high-density screens.
    const maxSide = Math.round(size * window.devicePixelRatio);
    // Masks are shown raw, layers as light: never the same thumbnail for one image.
    // Not kept while baking: the same layer bakes other content another time.
    const cacheKey = baking ? null : `${mask ? "mask" : "layer"}:${key}:${maxSide}`;
    const request = { documentId, layerId: layer.id, maxSide, mask, cacheKey };
    wanted = request;
    const known = cacheKey === null ? undefined : cache.get(cacheKey);
    if (known) {
      thumbnails.drop();
      draw(known);
      return;
    }
    // The first thumbnail at once, and a baking preview (rendered small by the GPU); new pixels
    // replace the one shown once they have rested.
    thumbnails.push(request, baking || !shown.has(slot));
  });

  $effect(() => () => {
    wanted = null;
    thumbnails.drop();
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
    <span class="checker fill">
      <span
        style:background={layer.gradientFill
          ? cssFill(document, layer.gradientFill)
          : swatch(layer.swatch)}
      ></span>
    </span>
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

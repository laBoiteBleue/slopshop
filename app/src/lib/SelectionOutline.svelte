<script lang="ts">
  // The marching ants (ADR 0024): the selection's outline, computed by the engine at the screen's
  // resolution over the visible area (plus a margin, so that small pans need nothing new), and
  // drawn here with an animated dash. A feathered selection also shows where its soft edge
  // starts and ends, dotted. The engine is asked again only when the selection, the level of
  // detail or the area changes enough.
  import { untrack } from "svelte";
  import { engine, type Polylines, type SelectionOutline } from "./engine";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    documentId,
    selectionKey,
    width,
    height,
  }: {
    mapping: ViewMapping;
    /** Read once: the overlay is recreated with the viewport for another document. */
    documentId: number;
    selectionKey: number;
    /** Canvas size, document pixels. */
    width: number;
    height: number;
  } = $props();

  const docId = untrack(() => documentId);
  let element: HTMLDivElement;
  let box = $state({ width: 0, height: 0 });

  type Fetched = {
    key: number;
    zoom: number;
    region: { x: number; y: number; width: number; height: number };
    outline: SelectionOutline;
  };
  let fetched = $state.raw<Fetched | null>(null);
  let request = 0;
  let timer = 0;

  /** Device pixels per document pixel, rounded down to a power of two (one pyramid level). */
  const zoom = $derived(2 ** Math.floor(Math.log2(window.devicePixelRatio / mapping.docPerCss)));

  /** The visible area, in document pixels, clamped to the canvas. */
  const visible = $derived.by(() => {
    if (!element || box.width === 0) return null;
    const rect = element.getBoundingClientRect();
    const [x0, y0] = mapping.toDocument(rect.left, rect.top);
    const [x1, y1] = mapping.toDocument(rect.left + box.width, rect.top + box.height);
    const left = Math.max(0, Math.floor(x0));
    const top = Math.max(0, Math.floor(y0));
    const right = Math.min(width, Math.ceil(x1));
    const bottom = Math.min(height, Math.ceil(y1));
    return { left, top, right, bottom };
  });

  $effect(() => {
    const key = selectionKey;
    const area = visible;
    const level = zoom;
    if (!area) return;
    const current = untrack(() => fetched);
    const covered =
      current &&
      current.key === key &&
      current.zoom === level &&
      area.left >= current.region.x &&
      area.top >= current.region.y &&
      area.right <= current.region.x + current.region.width &&
      area.bottom <= current.region.y + current.region.height;
    if (covered) return;
    // Half a view of margin on every side.
    const marginX = Math.ceil((area.right - area.left) / 2);
    const marginY = Math.ceil((area.bottom - area.top) / 2);
    const x = Math.max(0, area.left - marginX);
    const y = Math.max(0, area.top - marginY);
    const region = {
      x,
      y,
      width: Math.max(0, Math.min(width, area.right + marginX) - x),
      height: Math.max(0, Math.min(height, area.bottom + marginY) - y),
    };
    // A new selection is fetched at once; view changes wait for the view to settle a little.
    const delay = current?.key === key ? 60 : 0;
    window.clearTimeout(timer);
    const id = ++request;
    timer = window.setTimeout(() => {
      engine
        .selectionOutline(docId, region, level)
        .then((lines) => {
          if (id === request) fetched = { key, zoom: level, region, outline: lines };
        })
        .catch(() => undefined);
    }, delay);
  });

  $effect(() => () => window.clearTimeout(timer));

  /** Polylines as an SVG path in viewport pixels, on pixel centers for crisp lines. */
  function toPath(lines: Polylines): string {
    const parts: string[] = [];
    for (const line of lines) {
      for (let i = 0; i < line.length; i += 2) {
        const [x, y] = mapping.toViewport(line[i], line[i + 1]);
        parts.push(`${i === 0 ? "M" : "L"}${Math.round(x) + 0.5} ${Math.round(y) + 0.5}`);
      }
    }
    return parts.join("");
  }

  const current = $derived(fetched && fetched.key === selectionKey ? fetched.outline : null);
  const path = $derived(current ? toPath(current.middle) : "");
  const softPath = $derived(
    current?.soft ? toPath(current.soft.outer) + toPath(current.soft.inner) : "",
  );
</script>

<div
  class="outline"
  bind:this={element}
  bind:clientWidth={box.width}
  bind:clientHeight={box.height}
  aria-hidden="true"
>
  <svg>
    {#if softPath}
      <path class="soft-under" d={softPath} />
      <path class="soft" d={softPath} />
    {/if}
    <path class="under" d={path} />
    <path class="ants" d={path} />
  </svg>
</div>

<style>
  .outline {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  svg {
    display: block;
    width: 100%;
    height: 100%;
  }

  path {
    fill: none;
    stroke-width: 1;
  }

  .under {
    stroke: #ffffff;
  }

  .ants {
    stroke: #000000;
    stroke-dasharray: 4 4;
    animation: march 0.6s linear infinite;
  }

  /* The limits of a soft edge: dotted, still, fainter than the ants. */
  .soft-under {
    stroke: #ffffff;
    opacity: 0.6;
  }

  .soft {
    stroke: #000000;
    stroke-dasharray: 1 3;
    opacity: 0.8;
  }

  @keyframes march {
    to {
      stroke-dashoffset: -8;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ants {
      animation: none;
    }
  }
</style>

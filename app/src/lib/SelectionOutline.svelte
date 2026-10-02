<script lang="ts">
  // The marching ants (ADR 0024): the selection's outline, computed by the engine at the screen's
  // resolution over the visible area (plus a view of margin on every side, so that pans and
  // zooms out need nothing new for a while), and drawn here with an animated dash, where coverage
  // crosses one half (as Photoshop does; Quick Mask shows a soft edge). The engine is asked again
  // only when the selection, the level of detail or the area changes enough: at once, one
  // request at a time, the latest wins (as frames do); the last outline stays drawn meanwhile.
  // The SVG path is built for one view and follows pans and zooms with a transform, as the image
  // does while its frame renders: rebuilding it on every view change made the ants lag behind
  // the image. After a zoom, it is rebuilt once the view rests (crisp lines again).
  import { untrack } from "svelte";
  import { engine, type SelectionOutline } from "./engine";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    documentId,
    selectionKey,
    width,
    height,
    hidden = false,
  }: {
    mapping: ViewMapping;
    /** Read once: the overlay is recreated with the viewport for another document. */
    documentId: number;
    selectionKey: number;
    /** Canvas size, document pixels. */
    width: number;
    height: number;
    /** Not drawn (Quick Mask shows the selection): kept, so its outline is ready again. */
    hidden?: boolean;
  } = $props();

  const docId = untrack(() => documentId);
  let element: HTMLDivElement;
  let box = $state({ width: 0, height: 0 });

  type Region = { x: number; y: number; width: number; height: number };
  type Wanted = { key: number; zoom: number; region: Region };
  type Fetched = Wanted & { outline: SelectionOutline };
  let fetched = $state.raw<Fetched | null>(null);
  /** The request the engine is working on, and the latest one waiting for it. */
  let inFlight: Wanted | null = null;
  let waiting: Wanted | null = null;
  let destroyed = false;
  $effect(() => () => {
    destroyed = true;
  });

  type Area = { left: number; top: number; right: number; bottom: number };

  /** `w` is the outline wanted for `area` at `level` of selection `key`. */
  function covers(w: Wanted | null, key: number, level: number, area: Area): boolean {
    return (
      w !== null &&
      w.key === key &&
      w.zoom === level &&
      area.left >= w.region.x &&
      area.top >= w.region.y &&
      area.right <= w.region.x + w.region.width &&
      area.bottom <= w.region.y + w.region.height
    );
  }

  function send() {
    if (inFlight || !waiting || destroyed) return;
    const next = waiting;
    waiting = null;
    inFlight = next;
    engine
      .selectionOutline(docId, next.region, next.zoom)
      .then((outline) => {
        if (!destroyed) fetched = { ...next, outline };
      })
      .catch(() => undefined)
      .finally(() => {
        inFlight = null;
        send();
      });
  }

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
    if (!area || hidden) return;
    const current = untrack(() => fetched);
    if ([current, inFlight, waiting].some((w) => covers(w, key, level, area))) return;
    // A whole view of margin on every side.
    const marginX = area.right - area.left;
    const marginY = area.bottom - area.top;
    const x = Math.max(0, area.left - marginX);
    const y = Math.max(0, area.top - marginY);
    waiting = {
      key,
      zoom: level,
      region: {
        x,
        y,
        width: Math.max(0, Math.min(width, area.right + marginX) - x),
        height: Math.max(0, Math.min(height, area.bottom + marginY) - y),
      },
    };
    send();
  });

  /** A view as a viewport point is `document / docPerCss + offset` (CSS pixels). */
  type Placement = { docPerCss: number; offset: [number, number] };

  function placementOf(m: ViewMapping): Placement {
    return { docPerCss: m.docPerCss, offset: m.toViewport(0, 0) };
  }

  /** Polylines as an SVG path in viewport pixels, on pixel centers for crisp lines. */
  function toPath(lines: SelectionOutline, m: ViewMapping): string {
    const parts: string[] = [];
    for (const line of lines) {
      for (let i = 0; i < line.length; i += 2) {
        const [x, y] = m.toViewport(line[i], line[i + 1]);
        parts.push(`${i === 0 ? "M" : "L"}${Math.round(x) + 0.5} ${Math.round(y) + 0.5}`);
      }
    }
    return parts.join("");
  }

  const current = $derived(
    !hidden && fetched && fetched.key === selectionKey ? fetched.outline : null,
  );

  /** The path, and the view it was built for. */
  let built = $state.raw<{ outline: SelectionOutline; at: Placement; path: string } | null>(null);
  const rebuild = () => {
    const outline = current;
    built = outline ? { outline, at: placementOf(mapping), path: toPath(outline, mapping) } : null;
  };

  /** How the built path is moved and scaled to the current view. */
  const placed = $derived.by(() => {
    if (!built) return null;
    const now = placementOf(mapping);
    const k = built.at.docPerCss / now.docPerCss;
    // Whole device pixels, so that unscaled lines stay crisp.
    const dpr = window.devicePixelRatio;
    const snap = (v: number) => Math.round(v * dpr) / dpr;
    const tx = now.offset[0] - built.at.offset[0] * k;
    const ty = now.offset[1] - built.at.offset[1] * k;
    const scaled = Math.abs(k - 1) > 1e-9;
    return {
      scaled,
      transform: scaled
        ? `translate(${tx} ${ty}) scale(${k})`
        : `translate(${snap(tx)} ${snap(ty)})`,
    };
  });

  // A new outline: a new path, for the current view.
  $effect(() => {
    void current;
    untrack(rebuild);
  });

  // Zoomed: rebuilt once the view rests.
  $effect(() => {
    if (!placed?.scaled) return;
    const timer = setTimeout(rebuild, 150);
    return () => clearTimeout(timer);
  });
</script>

<div
  class="outline"
  bind:this={element}
  bind:clientWidth={box.width}
  bind:clientHeight={box.height}
  aria-hidden="true"
>
  <svg>
    {#if built && placed}
      <g transform={placed.transform}>
        <path class="under" d={built.path} vector-effect="non-scaling-stroke" />
        <path class="ants" d={built.path} vector-effect="non-scaling-stroke" />
      </g>
    {/if}
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

<script lang="ts">
  // The marching ants (ADR 0024): the selection's outline, computed by the engine at the screen's
  // resolution over the visible area (plus a view of margin on every side, so that pans and
  // zooms out need nothing new for a while), and drawn here with an animated dash, where coverage
  // crosses one half (as Photoshop does; Quick Mask shows a soft edge). The engine is asked again
  // only when the selection, the level of detail or the area changes enough: at once, one
  // request at a time, the latest wins (as frames do); the last outline stays drawn meanwhile.
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

  /** Polylines as an SVG path in viewport pixels, on pixel centers for crisp lines. */
  function toPath(lines: SelectionOutline): string {
    const parts: string[] = [];
    for (const line of lines) {
      for (let i = 0; i < line.length; i += 2) {
        const [x, y] = mapping.toViewport(line[i], line[i + 1]);
        parts.push(`${i === 0 ? "M" : "L"}${Math.round(x) + 0.5} ${Math.round(y) + 0.5}`);
      }
    }
    return parts.join("");
  }

  const current = $derived(
    !hidden && fetched && fetched.key === selectionKey ? fetched.outline : null,
  );
  const path = $derived(current ? toPath(current) : "");
</script>

<div
  class="outline"
  bind:this={element}
  bind:clientWidth={box.width}
  bind:clientHeight={box.height}
  aria-hidden="true"
>
  <svg>
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

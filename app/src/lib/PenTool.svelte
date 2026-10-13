<script lang="ts">
  // The Pen tool (P, ADR 0041), one tool where Photoshop has six. Each click adds a corner
  // anchor; a drag adds a smooth one, its handles opposite (Alt during the drag moves the
  // outgoing handle alone). Shift keeps the next anchor, or the handle, on 45° steps. A line
  // follows the pointer to show the next segment. A click on the first anchor closes the path
  // and Enter finishes an open one: either adds a vector layer, filled and stroked as the
  // options bar says. Backspace removes the last anchor, Esc drops the path. Ctrl+Enter makes
  // a selection of the path being drawn (dropping it), or else of the active vector layer's.
  import {
    closed,
    corner,
    dragged,
    geometryOf,
    isDrawable,
    nextPoint,
    rubberBand,
    svgOfPath,
    withAnchor,
    withoutLast,
    type Anchor,
    type PathGeometry,
    type PenPath,
    type Point,
  } from "./pen";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    onpath,
    onmakeselection,
  }: {
    mapping: ViewMapping;
    /** A path was finished (closed, or Enter), in document pixels. */
    onpath: (geometry: PathGeometry) => void;
    /** Ctrl+Enter: a selection of `geometry` (the path being drawn), or with `null` of the
     *  active layer's path. */
    onmakeselection: (geometry: PathGeometry | null) => void;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click: a corner anchor. */
  const CLICK_SLOP = 3;
  /** A press this close (CSS pixels) to the first anchor closes the path. */
  const CLOSE_RADIUS = 6;

  type Drag = {
    pointerId: number;
    /** The anchor being placed (or the first one, when closing). */
    point: Point;
    client: [number, number];
    moved: boolean;
    /** Alt was pressed during the drag: the incoming handle stays here. */
    frozenIn: Point | null;
    /** The press was on the first anchor: the release closes the path. */
    closing: boolean;
    /** The anchor as the drag shapes it. */
    anchor: Anchor;
  };

  let path = $state<PenPath | null>(null);
  let drag = $state<Drag | null>(null);
  /** The pointer, in document pixels and in viewport pixels. */
  let pointer = $state<{ doc: Point; view: Point } | null>(null);
  let shift = $state(false);
  let element: SVGSVGElement;

  function locate(e: PointerEvent): { doc: Point; view: Point } {
    const box = element.getBoundingClientRect();
    return {
      doc: mapping.toDocument(e.clientX, e.clientY),
      view: [e.clientX - box.left, e.clientY - box.top],
    };
  }

  function screenDistance(a: Point, b: Point): number {
    const [ax, ay] = mapping.toViewport(a[0], a[1]);
    const [bx, by] = mapping.toViewport(b[0], b[1]);
    return Math.hypot(ax - bx, ay - by);
  }

  /** Whether a press at `at` lands on the first anchor of a path that can close. */
  function onFirst(at: Point): boolean {
    return (
      path !== null &&
      path.anchors.length >= 2 &&
      screenDistance(path.anchors[0].point, at) <= CLOSE_RADIUS
    );
  }

  function finish(done: PenPath | null) {
    path = null;
    drag = null;
    if (isDrawable(done)) onpath(geometryOf(done));
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture?.(e.pointerId);
    const at = locate(e);
    pointer = at;
    const closing = onFirst(at.doc);
    const point = closing && path ? path.anchors[0].point : nextPoint(path, at.doc, e.shiftKey);
    const anchor = closing && path ? path.anchors[0] : corner(point);
    if (!closing) path = withAnchor(path, anchor);
    drag = {
      pointerId: e.pointerId,
      point,
      client: [e.clientX, e.clientY],
      moved: false,
      frozenIn: null,
      closing,
      anchor,
    };
  }

  function move(e: PointerEvent) {
    shift = e.shiftKey;
    pointer = locate(e);
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    const moved =
      current.moved ||
      Math.hypot(e.clientX - current.client[0], e.clientY - current.client[1]) >= CLICK_SLOP;
    if (!moved) return;
    // Alt pressed during the drag: the incoming handle stays where it is.
    const frozenIn = current.frozenIn ?? (e.altKey && current.moved ? current.anchor.in : null);
    const anchor = dragged(current.point, pointer.doc, e.shiftKey, frozenIn);
    drag = { ...current, moved, frozenIn, anchor };
    if (!current.closing && path) {
      path = { ...path, anchors: [...path.anchors.slice(0, -1), anchor] };
    }
  }

  function end(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    if (current.closing && path) finish(closed(path, current.moved ? current.anchor : undefined));
  }

  // The Pen's keys, before the app's own shortcuts see them.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      shift = e.shiftKey;
      if (e.type !== "keydown") return;
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        const drawn = path;
        path = null;
        drag = null;
        onmakeselection(isDrawable(drawn) ? geometryOf(drawn) : null);
      } else if (!path) {
        return;
      } else if (e.key === "Enter") {
        finish(path);
      } else if (e.key === "Escape") {
        path = null;
        drag = null;
      } else if (e.key === "Backspace" || e.key === "Delete") {
        if (drag) return;
        path = withoutLast(path);
      } else {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener("keyup", onKey, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener("keyup", onKey, { capture: true });
    };
  });

  const toViewport = (x: number, y: number): Point => mapping.toViewport(x, y);

  /** What is drawn over the image: the path, the next segment, anchors and handles. */
  const shown = $derived.by(() => {
    if (!path) return null;
    const closing = drag?.closing ? closed(path, drag.moved ? drag.anchor : undefined) : null;
    const outline = svgOfPath(geometryOf(closing ?? path), toViewport);
    const band =
      !drag && pointer
        ? rubberBand(
            path,
            onFirst(pointer.doc) ? path.anchors[0].point : nextPoint(path, pointer.doc, shift),
          )
        : null;
    const anchors = (closing ?? path).anchors;
    // The handles of the anchor being placed, or of the last one.
    const handled = drag ? drag.anchor : anchors[anchors.length - 1];
    const handles = [handled.in, handled.out]
      .filter((h) => h[0] !== handled.point[0] || h[1] !== handled.point[1])
      .map((h) => toViewport(h[0], h[1]));
    return {
      outline,
      band: band ? svgOfPath(band, toViewport) : null,
      anchors: anchors.map((a) => toViewport(a.point[0], a.point[1])),
      handled: toViewport(handled.point[0], handled.point[1]),
      handles,
      closingHint: !drag && pointer !== null && onFirst(pointer.doc),
    };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="pen-tool"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={begin}
  onpointermove={move}
  onpointerleave={() => (pointer = null)}
  onpointerup={end}
  onpointercancel={() => (drag = null)}
>
  {#if shown}
    <path class="under" d={shown.outline} data-testid="pen-path" />
    <path class="outline" d={shown.outline} />
    {#if shown.band}
      <path class="band" d={shown.band} data-testid="pen-band" />
    {/if}
    {#each shown.handles as [x, y], i (i)}
      <line class="handle" x1={shown.handled[0]} y1={shown.handled[1]} x2={x} y2={y} />
      <circle class="knob" cx={x} cy={y} r="3" />
    {/each}
    {#each shown.anchors as [x, y], i (i)}
      <rect class="anchor" x={x - 3} y={y - 3} width="6" height="6" data-testid="pen-anchor" />
    {/each}
    {#if shown.closingHint && pointer}
      <!-- On the first anchor: a click closes the path. -->
      <circle class="close" cx={pointer.view[0] + 10} cy={pointer.view[1] + 10} r="3" />
    {/if}
  {/if}
</svg>

<style>
  .pen-tool {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .pen-tool.hand {
    pointer-events: none;
  }

  .under,
  .outline,
  .band {
    fill: none;
    stroke-linejoin: round;
  }

  .under {
    stroke: #000000;
    stroke-width: 3;
    opacity: 0.6;
  }

  .outline {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .band {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
    stroke-dasharray: 4 3;
  }

  .handle {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .knob {
    fill: var(--accent, #3b82f6);
  }

  .anchor {
    fill: #ffffff;
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .close {
    fill: none;
    stroke: #ffffff;
    stroke-width: 1.5;
    paint-order: stroke;
  }
</style>

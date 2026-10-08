<script lang="ts">
  // The Pen (P, ADR 0041), as Photoshop's in Shape mode: a click places a corner, a press and
  // drag a smooth anchor (Alt while dragging moves the handle after it only), Shift keeps the
  // new anchor or the handle at 45° steps; a click on the first anchor closes the path. Enter or
  // Esc ends an open path, Backspace takes back the last anchor; a closed or ended path becomes
  // a vector layer. Changing tool ends the path too.
  import { onDestroy } from "svelte";
  import { snapped45 } from "./gradient";
  import {
    HIT_PIXELS,
    drawable,
    geometryOf,
    press,
    pull,
    svgOf,
    withoutLast,
    type PenPath,
    type PathGeometry,
    type Point,
  } from "./pen";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    onpath,
  }: {
    mapping: ViewMapping;
    /** A path was drawn, in document pixels. */
    onpath: (geometry: PathGeometry) => void;
  } = $props();

  /** A press moving less than this (CSS pixels) places a corner, not a smooth anchor. */
  const CLICK_SLOP = 3;

  let path = $state<PenPath | null>(null);
  /** The press being dragged: where it was on screen, and whether it moved enough to pull. */
  let drag = $state<{ pointerId: number; client: Point; pulled: boolean } | null>(null);
  /** The pointer over the image, for the segment the next click would add. */
  let hover = $state<Point | null>(null);
  let shift = $state(false);

  const reach = $derived(HIT_PIXELS * mapping.docPerCss);

  function last(p: PenPath): Point {
    return p.anchors[p.anchors.length - 1].point;
  }

  /** The path drawn so far becomes a layer, if it draws something. */
  function finish() {
    const done = path;
    path = null;
    drag = null;
    if (done && drawable(done)) onpath(geometryOf([done]));
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    let at = mapping.toDocument(e.clientX, e.clientY);
    if (path && e.shiftKey) at = snapped45(last(path), at, true);
    path = press(path, at, reach);
    drag = { pointerId: e.pointerId, client: [e.clientX, e.clientY], pulled: false };
  }

  function update(e: PointerEvent) {
    shift = e.shiftKey;
    const at = mapping.toDocument(e.clientX, e.clientY);
    hover = at;
    if (!drag || e.pointerId !== drag.pointerId || !path) return;
    const pulled =
      drag.pulled ||
      Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]) >= CLICK_SLOP;
    if (!pulled) return;
    drag = { ...drag, pulled };
    const anchor = path.closed ? path.anchors[0].point : last(path);
    path = pull(path, snapped45(anchor, at, e.shiftKey), e.altKey);
  }

  function end(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    drag = null;
    if (path?.closed) finish();
  }

  // Keys of the Pen while it draws, before the app's own shortcuts see them (Delete would
  // open Fill, Esc would drop the selection).
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      shift = e.shiftKey;
      if (e.type !== "keydown" || !path || e.target instanceof HTMLInputElement) return;
      if (e.key === "Enter" || e.key === "Escape") {
        finish();
      } else if (e.key === "Backspace" || e.key === "Delete") {
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

  onDestroy(finish);

  const shown = $derived.by(() => {
    if (!path) return null;
    const d = svgOf([path], mapping.toViewport);
    const anchors = path.anchors.map((a) => mapping.toViewport(a.point[0], a.point[1]));
    const handles = path.anchors.flatMap((a) =>
      [a.before, a.after]
        .filter((h): h is Point => h !== null)
        .map((h) => ({
          from: mapping.toViewport(a.point[0], a.point[1]),
          to: mapping.toViewport(h[0], h[1]),
        })),
    );
    // The segment the next click would add, straight from the last anchor.
    let next: string | null = null;
    if (!drag && !path.closed && hover) {
      const [x0, y0] = mapping.toViewport(...last(path));
      const to = shift ? snapped45(last(path), hover, true) : hover;
      const [x1, y1] = mapping.toViewport(to[0], to[1]);
      next = `M${x0.toFixed(2)} ${y0.toFixed(2)} L${x1.toFixed(2)} ${y1.toFixed(2)}`;
    }
    return { d, anchors, handles, next };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="pen-tool"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={begin}
  onpointermove={update}
  onpointerup={end}
  onpointerleave={() => (hover = null)}
  onpointercancel={() => (drag = null)}
>
  {#if shown}
    <path class="under" d={shown.d} />
    <path class="outline" d={shown.d} />
    {#if shown.next}
      <path class="next" d={shown.next} />
    {/if}
    {#each shown.handles as handle, i (i)}
      <line
        class="handle-line"
        x1={handle.from[0]}
        y1={handle.from[1]}
        x2={handle.to[0]}
        y2={handle.to[1]}
      />
      <circle class="handle" cx={handle.to[0]} cy={handle.to[1]} r="3" />
    {/each}
    {#each shown.anchors as [x, y], i (i)}
      <rect class="anchor" class:first={i === 0} x={x - 3} y={y - 3} width="6" height="6" />
    {/each}
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
  .next {
    fill: none;
  }

  .under {
    stroke: #000000;
    stroke-width: 3;
    opacity: 0.6;
  }

  .outline,
  .next,
  .handle-line {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .next {
    stroke-dasharray: 4 3;
  }

  .handle {
    fill: var(--accent, #3b82f6);
  }

  .anchor {
    fill: var(--accent, #3b82f6);
    stroke: #ffffff;
    stroke-width: 1;
  }

  /* Where a click closes the path. */
  .anchor.first {
    fill: #ffffff;
    stroke: var(--accent, #3b82f6);
  }
</style>

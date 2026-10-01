<script lang="ts">
  // The Lasso and Polygonal Lasso tools (L, ADR 0024). Lasso: drag on the image to draw a
  // freehand outline; releasing closes it with a straight line back to its start. Polygonal
  // Lasso: each click adds a corner, a line follows the pointer (Shift: by steps of 45°); a click
  // on the first corner, a double-click or Enter closes the shape, Backspace removes the last
  // corner, Esc drops it. As with the marquees: Shift, Alt or both at the first press add,
  // subtract or intersect (else the options bar's mode), a +, − or × badge by the pointer says
  // which, and a click without drawing deselects. Points keep their fractions: the engine gives
  // edge pixels their exact coverage (anti-alias) or not.
  import type { SelectionMode, SelectionShape } from "./engine";
  import { MODE_BADGES, modeFromKeys } from "./selection";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    polygonal,
    mode,
    onselect,
    ondeselect,
  }: {
    mapping: ViewMapping;
    /** Polygonal Lasso: corners by clicks; otherwise freehand. */
    polygonal: boolean;
    /** The options bar's mode, which keys override. */
    mode: SelectionMode;
    onselect: (shape: SelectionShape, mode: SelectionMode | null) => void;
    /** A click without drawing, without keys. */
    ondeselect: () => void;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click. */
  const CLICK_SLOP = 3;
  /** A click this close (CSS pixels) to the first corner closes the polygon. */
  const CLOSE_RADIUS = 6;
  /** Freehand points closer than this (CSS pixels) to the last one are skipped. */
  const MIN_STEP = 0.75;

  type Drawing = {
    /** Document points so far. */
    points: [number, number][];
    mode: SelectionMode | null;
    /** Freehand: the pointer drawing, and whether it moved enough to draw. */
    pointerId: number | null;
    moved: boolean;
    start: [number, number];
  };
  let drawing = $state<Drawing | null>(null);
  /** The pointer, in document pixels and in viewport pixels. */
  let pointer = $state<{ doc: [number, number]; view: [number, number] } | null>(null);
  let keys = $state({ shiftKey: false, altKey: false });
  let element: SVGSVGElement;

  const badge = $derived(
    MODE_BADGES[drawing ? (drawing.mode ?? mode) : (modeFromKeys(keys) ?? mode)],
  );

  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }

  function locate(e: PointerEvent): { doc: [number, number]; view: [number, number] } {
    const box = element.getBoundingClientRect();
    return {
      doc: mapping.toDocument(e.clientX, e.clientY),
      view: [e.clientX - box.left, e.clientY - box.top],
    };
  }

  /** `to` from `from` along the nearest multiple of 45° (Shift on the Polygonal Lasso). */
  function constrained(from: [number, number], to: [number, number]): [number, number] {
    const dx = to[0] - from[0];
    const dy = to[1] - from[1];
    const step = Math.PI / 4;
    const angle = Math.round(Math.atan2(dy, dx) / step) * step;
    const length = Math.hypot(dx, dy);
    return [from[0] + Math.cos(angle) * length, from[1] + Math.sin(angle) * length];
  }

  /** Where the next corner goes: the pointer, constrained with Shift. */
  function nextCorner(doc: [number, number], shift: boolean): [number, number] {
    const last = drawing?.points.at(-1);
    return last && shift ? constrained(last, doc) : doc;
  }

  function finish() {
    const current = drawing;
    drawing = null;
    if (!current) return;
    if (current.points.length >= 3) {
      onselect({ kind: "polygon", points: current.points }, current.mode);
    }
  }

  function screenDistance(a: [number, number], b: [number, number]): number {
    const [ax, ay] = mapping.toViewport(a[0], a[1]);
    const [bx, by] = mapping.toViewport(b[0], b[1]);
    return Math.hypot(ax - bx, ay - by);
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    // The press keeps the focus where it was: a field being typed in (Feather) is done.
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    const at = locate(e);
    pointer = at;
    if (polygonal) {
      if (!drawing) {
        drawing = {
          points: [at.doc],
          mode: modeFromKeys(e),
          pointerId: null,
          moved: false,
          start: [e.clientX, e.clientY],
        };
        return;
      }
      const first = drawing.points[0];
      if (drawing.points.length >= 3 && screenDistance(first, at.doc) <= CLOSE_RADIUS) {
        finish();
        return;
      }
      drawing.points.push(nextCorner(at.doc, e.shiftKey));
      return;
    }
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drawing = {
      points: [at.doc],
      mode: modeFromKeys(e),
      pointerId: e.pointerId,
      moved: false,
      start: [e.clientX, e.clientY],
    };
  }

  function move(e: PointerEvent) {
    track(e);
    pointer = locate(e);
    const current = drawing;
    if (!current || polygonal || e.pointerId !== current.pointerId) return;
    if (!current.moved) {
      const distance = Math.hypot(e.clientX - current.start[0], e.clientY - current.start[1]);
      if (distance < CLICK_SLOP) return;
      current.moved = true;
    }
    // Every intermediate position the system reported, not only the latest.
    const events = e.getCoalescedEvents?.() ?? [];
    for (const event of events.length > 0 ? events : [e]) {
      const [x, y] = mapping.toDocument(event.clientX, event.clientY);
      const last = current.points[current.points.length - 1];
      if (screenDistance(last, [x, y]) >= MIN_STEP) current.points.push([x, y]);
    }
  }

  function end(e: PointerEvent) {
    const current = drawing;
    if (!current || polygonal || e.pointerId !== current.pointerId) return;
    if (!current.moved) {
      drawing = null;
      if (current.mode === null) ondeselect();
      return;
    }
    finish();
  }

  function onDoubleClick() {
    // The double-click's two presses added the same corner twice: drop the repeat.
    if (!polygonal || !drawing) return;
    const points = drawing.points;
    if (
      points.length >= 2 &&
      screenDistance(points[points.length - 1], points[points.length - 2]) < 1
    ) {
      points.pop();
    }
    finish();
  }

  // Keys of the Polygonal Lasso while it draws, before the app's own shortcuts see them.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      track(e);
      if (e.type !== "keydown" || !drawing || !polygonal) return;
      if (e.key === "Enter") {
        finish();
      } else if (e.key === "Escape") {
        drawing = null;
      } else if (e.key === "Backspace" || e.key === "Delete") {
        if (drawing.points.length > 1) drawing.points.pop();
        else drawing = null;
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

  /** The outline drawn so far, in viewport pixels, with the line to the pointer. */
  const preview = $derived.by(() => {
    if (!drawing) return null;
    const points = drawing.points.map(([x, y]) => mapping.toViewport(x, y));
    if (polygonal && pointer) {
      const next = nextCorner(pointer.doc, keys.shiftKey);
      points.push(mapping.toViewport(next[0], next[1]));
    }
    if (!polygonal && !drawing.moved) return null;
    const closing =
      polygonal &&
      pointer !== null &&
      drawing.points.length >= 3 &&
      screenDistance(drawing.points[0], pointer.doc) <= CLOSE_RADIUS;
    return { points: points.map(([x, y]) => `${x},${y}`).join(" "), closing };
  });
</script>

<svg
  class="lasso"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={begin}
  onpointermove={move}
  onpointerleave={() => (pointer = null)}
  onpointerup={end}
  onpointercancel={() => {
    if (!polygonal) drawing = null;
  }}
  ondblclick={onDoubleClick}
>
  {#if preview}
    <polyline class="under" points={preview.points} />
    <polyline class="ants" points={preview.points} />
    {#if preview.closing && pointer}
      <!-- On the first corner: a click closes the shape. -->
      <circle class="close" cx={pointer.view[0] + 10} cy={pointer.view[1] + 10} r="3" />
    {/if}
  {/if}
  {#if pointer && badge && !mapping.hand}
    <text class="badge" x={pointer.view[0] + 8} y={pointer.view[1] + 16}>{badge}</text>
  {/if}
</svg>

<style>
  .lasso {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .lasso.hand {
    pointer-events: none;
  }

  .under,
  .ants {
    fill: none;
    stroke-width: 1;
    stroke-linejoin: round;
  }

  .under {
    stroke: #ffffff;
  }

  .ants {
    stroke: #000000;
    stroke-dasharray: 4 4;
    animation: march 0.6s linear infinite;
  }

  .close {
    fill: none;
    stroke: #ffffff;
    stroke-width: 1.5;
    paint-order: stroke;
  }

  .badge {
    fill: #ffffff;
    font-size: 13px;
    font-weight: 700;
    paint-order: stroke;
    stroke: #000000;
    stroke-width: 3px;
    pointer-events: none;
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

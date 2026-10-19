<script lang="ts">
  // Edit > Perspective Warp (ADR 0044) over the image, as in Photoshop. Layout mode: a drag
  // draws a plane (a rectangle; its corners snap to other planes' corners nearby), a corner's
  // drag fits it to the image's lines (dropped by another plane's corner, it joins it), a click
  // selects a plane and Delete removes it. Warp mode: a corner's drag moves it, the image
  // following (each change sent live by the parent). Enter or a double-click applies (one undo
  // entry), Esc cancels.
  import * as homography from "./homography";
  import type { Homography } from "./homography";
  import { isTextField } from "./keymap";
  import {
    addPlane,
    joinCorner,
    moveCorner,
    planeAt,
    planeLines,
    removePlane,
    type PerspectiveMode,
    type Planes,
    type Point,
  } from "./perspectiveWarp";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    planes,
    mode,
    toDocument,
    selected = null,
    onchange,
    onselect = () => {},
    oncommit,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The planes, in the layer's content pixels. */
    planes: Planes;
    mode: PerspectiveMode;
    /** The map from the layer's content to the document. */
    toDocument: Homography;
    /** The plane selected (Delete removes it in Layout mode). */
    selected?: number | null;
    /** The planes as edited (a drag's step, a plane drawn, joined or removed). */
    onchange: (planes: Planes) => void;
    onselect?: (plane: number | null) => void;
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  /** Corners within this distance (CSS pixels) of the pointer are grabbed. */
  const GRAB = 6;
  /** Corners within this distance (CSS pixels) of one another snap together. */
  const SNAP = 10;

  const toContent = $derived(homography.invert(toDocument));
  const viewport = (p: Point): Point =>
    mapping.toViewport(...homography.apply(toDocument, p[0], p[1]));
  const corners = $derived((mode === "layout" ? planes.layout : planes.warped).map(viewport));
  const lines = $derived(
    planes.planes.map((_, k) =>
      planeLines(planes, k, mode).map(([a, b]) => [viewport(a), viewport(b)] as [Point, Point]),
    ),
  );

  type Drag =
    | { kind: "corner"; pointerId: number; index: number; start: Planes; from: Point }
    | { kind: "draw"; pointerId: number; from: Point; to: Point; moved: boolean };
  let drag = $state<Drag | null>(null);

  /** The content point under a window point, if the layer's map can be inverted. */
  function contentAt(e: PointerEvent): Point | null {
    if (!toContent) return null;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    const p = homography.apply(toContent, x, y);
    return Number.isFinite(p[0]) && Number.isFinite(p[1]) ? p : null;
  }

  /** SNAP in content pixels around content point `p`. */
  function snapAt(p: Point): number {
    const [ax, ay] = viewport(p);
    const [bx, by] = viewport([p[0] + 1, p[1]]);
    const cssPerContent = Math.hypot(bx - ax, by - ay);
    return cssPerContent > 0 ? SNAP / cssPerContent : 0;
  }

  /** The corner nearest to viewport point (`x`, `y`) within reach. */
  function grabbed(x: number, y: number): number | null {
    let best: number | null = null;
    let bestDistance = GRAB;
    corners.forEach(([cx, cy], i) => {
      const d = Math.hypot(cx - x, cy - y);
      if (d < bestDistance) {
        bestDistance = d;
        best = i;
      }
    });
    return best;
  }

  function onPointerDown(e: PointerEvent) {
    // Other buttons, or Space held: the viewport pans.
    if (e.button !== 0 || mapping.hand) return;
    const from = contentAt(e);
    if (!from) return;
    e.stopPropagation();
    e.preventDefault();
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    const index = grabbed(e.clientX - rect.left, e.clientY - rect.top);
    if (index !== null) {
      drag = { kind: "corner", pointerId: e.pointerId, index, start: planes, from };
    } else if (mode === "layout") {
      drag = { kind: "draw", pointerId: e.pointerId, from, to: from, moved: false };
    } else {
      onselect(planeAt(planes, mode, from));
      return;
    }
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const to = contentAt(e);
    if (!to) return;
    if (drag.kind === "draw") {
      drag = { ...drag, to, moved: drag.moved || to[0] !== drag.from[0] || to[1] !== drag.from[1] };
      return;
    }
    const [dx, dy] = [to[0] - drag.from[0], to[1] - drag.from[1]];
    if (dx === 0 && dy === 0) return;
    const start = mode === "layout" ? drag.start.layout : drag.start.warped;
    const [sx, sy] = start[drag.index];
    const next = moveCorner(drag.start, drag.index, [sx + dx, sy + dy], mode);
    if (next) onchange(next);
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const done = drag;
    drag = null;
    if (done.kind === "corner") {
      if (mode !== "layout") return;
      const at = planes.layout[done.index];
      const joined = at ? joinCorner(planes, done.index, snapAt(at)) : planes;
      if (joined !== planes) onchange(joined);
      return;
    }
    if (!done.moved) {
      onselect(planeAt(planes, mode, done.from));
      return;
    }
    const next = addPlane(planes, done.from, done.to, snapAt(done.from));
    if (next) {
      onchange(next);
      onselect(next.planes.length - 1);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (isTextField(e.target)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      oncommit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    }
  }

  // Delete and Backspace remove the selected plane in Layout mode, and never reach the window's
  // shortcuts (deleting layers) while the planes show: listened to first, as the Pen does.
  $effect(() => {
    const onDelete = (e: KeyboardEvent) => {
      if ((e.key !== "Delete" && e.key !== "Backspace") || isTextField(e.target)) return;
      e.preventDefault();
      e.stopPropagation();
      if (mode !== "layout" || selected === null || selected >= planes.planes.length) return;
      onchange(removePlane(planes, selected));
      onselect(null);
    };
    window.addEventListener("keydown", onDelete, { capture: true });
    return () => window.removeEventListener("keydown", onDelete, { capture: true });
  });

  /** The rectangle being drawn, in the viewport. */
  const drawn = $derived.by(() => {
    if (drag?.kind !== "draw" || !drag.moved) return null;
    const [[x0, y0], [x1, y1]] = [drag.from, drag.to];
    const quad: Point[] = [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ];
    return quad
      .map(viewport)
      .map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`)
      .join(" ");
  });
</script>

<svelte:window onkeydown={onKeydown} />

<svg
  class="perspective-warp"
  class:layout={mode === "layout"}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={() => (drag = null)}
  ondblclick={() => oncommit()}
>
  {#each lines as plane, k (k)}
    <g class="plane" class:selected={selected === k}>
      {#each plane as [a, b], i (i)}
        <line class:edge={i < 4} x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} />
      {/each}
    </g>
  {/each}
  {#if drawn}
    <polygon class="drawn" points={drawn} />
  {/if}
  {#each corners as [x, y], i (i)}
    <circle class="pin" cx={x} cy={y} r="4" />
  {/each}
</svg>

<style>
  .perspective-warp {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .perspective-warp.layout {
    cursor: crosshair;
  }

  line {
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    opacity: 0.6;
  }

  line.edge {
    stroke-width: 1.5;
    opacity: 1;
  }

  .plane.selected line.edge {
    stroke-width: 2.5;
  }

  .drawn {
    fill: none;
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    stroke-dasharray: 4 3;
  }

  .pin {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }
</style>

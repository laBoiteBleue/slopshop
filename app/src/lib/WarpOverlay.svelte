<script lang="ts">
  // Edit > Transform > Warp (ADR 0044), in Free Transform's place over the image: the layer's
  // warp grid, its anchors and their handles. Dragging an anchor (its handles with it) or a
  // handle reshapes the grid, dragging inside a patch bends it, as in Photoshop; with a Split
  // Warp mode on, a click splits the grid there. Each change is sent as the whole grid (live);
  // Enter or a double-click applies (one undo entry), Esc cancels, as in Free Transform.
  import * as homography from "./homography";
  import type { Homography } from "./homography";
  import { isTextField } from "./keymap";
  import type { ViewMapping } from "./Viewport.svelte";
  import { linePath } from "./warp";
  import {
    anchors,
    bendPatch,
    gridLines,
    handles,
    moveAnchor,
    moveHandle,
    parameterAt,
    split as splitGrid,
    type Grid,
    type Point,
    type SplitMode,
  } from "./warpGrid";

  let {
    mapping,
    grid,
    toDocument,
    split = null,
    onchange,
    oncommit,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The grid, its points in the layer's content pixels. */
    grid: Grid;
    /** The map from the layer's content to the document. */
    toDocument: Homography;
    /** Split Warp's mode: a click splits the grid there. */
    split?: SplitMode | null;
    /** The grid as edited (a drag's step, or a split). */
    onchange: (grid: Grid) => void;
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  /** Points in the viewport within this distance (CSS pixels) of the pointer are grabbed. */
  const GRAB = 6;

  const toContent = $derived(homography.invert(toDocument));
  const screen = $derived(
    grid.points.map(([x, y]) => mapping.toViewport(...homography.apply(toDocument, x, y))),
  );
  const paths = $derived(
    gridLines(grid).map((line) =>
      linePath(
        line.map(([x, y]) => homography.apply(toDocument, x, y)),
        mapping.toViewport,
      ),
    ),
  );
  const handleList = $derived(handles(grid));
  const anchorList = $derived(anchors(grid));

  type Grab = { kind: "anchor"; index: number } | { kind: "handle"; index: number };
  type Drag = { pointerId: number; start: Grid; from: Point } & (
    Grab | { kind: "bend"; s: number; t: number }
  );
  let drag: Drag | null = null;

  /** The content point under a window point, if the layer's map can be inverted. */
  function contentAt(e: PointerEvent): Point | null {
    if (!toContent) return null;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    const p = homography.apply(toContent, x, y);
    return Number.isFinite(p[0]) && Number.isFinite(p[1]) ? p : null;
  }

  /** The anchor or handle nearest to viewport point (`x`, `y`) within reach; anchors first. */
  function grabbed(x: number, y: number): Grab | null {
    let best: Grab | null = null;
    let bestDistance = GRAB;
    const consider = (kind: Grab["kind"], index: number) => {
      const d = Math.hypot(screen[index][0] - x, screen[index][1] - y);
      if (d < bestDistance) {
        bestDistance = d;
        best = { kind, index };
      }
    };
    for (const index of anchorList) consider("anchor", index);
    for (const { handle } of handleList) consider("handle", handle);
    return best;
  }

  function onPointerDown(e: PointerEvent) {
    // Other buttons, or Space held: the viewport pans.
    if (e.button !== 0 || mapping.hand) return;
    const from = contentAt(e);
    if (!from) return;
    e.stopPropagation();
    e.preventDefault();
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    if (split) {
      const at = parameterAt(grid, toDocument, x, y);
      const next = at && splitGrid(grid, split, at[0], at[1]);
      if (next) onchange(next);
      return;
    }
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    const point = grabbed(e.clientX - rect.left, e.clientY - rect.top);
    const base = { pointerId: e.pointerId, start: grid, from };
    if (point) {
      drag = { ...base, ...point };
    } else {
      const at = parameterAt(grid, toDocument, x, y);
      if (!at) return;
      drag = { ...base, kind: "bend", s: at[0], t: at[1] };
    }
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const to = contentAt(e);
    if (!to) return;
    const [dx, dy] = [to[0] - drag.from[0], to[1] - drag.from[1]];
    if (dx === 0 && dy === 0) return;
    onchange(
      drag.kind === "anchor"
        ? moveAnchor(drag.start, drag.index, dx, dy)
        : drag.kind === "handle"
          ? moveHandle(drag.start, drag.index, dx, dy)
          : bendPatch(drag.start, drag.s, drag.t, dx, dy),
    );
  }

  function onPointerUp(e: PointerEvent) {
    if (drag && e.pointerId === drag.pointerId) drag = null;
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
</script>

<svelte:window onkeydown={onKeydown} />

<svg
  class="warp-grid"
  class:splitting={split !== null}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  ondblclick={() => oncommit()}
>
  {#each paths as d, i (i)}
    <path class="line" {d} />
  {/each}
  {#each handleList as { handle, anchor } (handle)}
    <line
      class="arm"
      x1={screen[anchor][0]}
      y1={screen[anchor][1]}
      x2={screen[handle][0]}
      y2={screen[handle][1]}
    />
    <circle class="handle" cx={screen[handle][0]} cy={screen[handle][1]} r="3" />
  {/each}
  {#each anchorList as index (index)}
    <rect
      class="anchor"
      x={screen[index][0] - 3.5}
      y={screen[index][1] - 3.5}
      width="7"
      height="7"
    />
  {/each}
</svg>

<style>
  .warp-grid {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .warp-grid.splitting {
    cursor: crosshair;
  }

  .line {
    fill: none;
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .arm {
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    opacity: 0.7;
  }

  .handle {
    fill: var(--accent, #2d8cff);
    stroke: #fff;
    stroke-width: 1;
  }

  .anchor {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }
</style>

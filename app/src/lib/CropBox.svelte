<script lang="ts">
  // The Crop tool (C, as in Photoshop): a frame on the image with eight handles, the outside
  // shaded and the rule of thirds inside. Drag inside to move it, a handle to resize it (Shift on
  // a corner keeps the proportions; a ratio from the options bar is kept by every handle; a size
  // fixes the frame, which then only moves), outside to draw a new one. With Straighten (the
  // options bar), a drag draws a line along what should be level or upright, and the image
  // turns. Edges snap to the canvas and the
  // other layers, and to their sizes. Enter, a double-click inside or a click outside (without
  // dragging) applies; Esc cancels. The frame stays on whole document pixels: cropping never
  // resamples, and nothing is deleted (ADR 0017).
  import { untrack } from "svelte";
  import {
    aspectRatio,
    centered,
    fitRatio,
    keepRatio,
    straightenTurn,
    type CropAspect,
  } from "./crop";
  import type { Bounds } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { hasShortcutModifier } from "./platform";
  import { SNAP_CSS_PX, snapHandle, snapMove, type SmartGuide, type SnapTarget } from "./snap";
  import type { ViewMapping } from "./Viewport.svelte";
  import SmartGuides from "./SmartGuides.svelte";

  let {
    mapping,
    canvas,
    targets = [],
    smartGuides = true,
    aspect = { mode: "free" },
    start,
    straighten = false,
    onstraighten,
    onapply,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The canvas, where the frame starts. */
    canvas: Bounds;
    /** What the frame's edges snap to; none when snapping is off. Ctrl held: no snapping. */
    targets?: SnapTarget[];
    /** The snaps' smart guides are drawn (View > Hide Extras hides them; the snap stays). */
    smartGuides?: boolean;
    /** The options bar's ratio or size, if any. */
    aspect?: CropAspect;
    /** The frame to start with (after Straighten), else the canvas. */
    start?: Bounds;
    /** Straighten is on: a drag draws a line; `onstraighten` turns the image (degrees, clockwise). */
    straighten?: boolean;
    onstraighten?: (degrees: number) => void;
    /** Crop to `frame` (whole document pixels). */
    onapply: (frame: Bounds) => void;
    oncancel: () => void;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click, not a drag. */
  const CLICK_SLOP = 3;

  // Starts on the canvas as it is when the tool opens.
  let frame = $state<Bounds>(untrack(() => ({ ...(start ?? canvas) })));
  /** Straighten's line while it is drawn, document pixels. */
  let line = $state<{ from: [number, number]; to: [number, number] } | null>(null);
  let guides = $state<SmartGuide[]>([]);
  let readout = $state<{ text: string; x: number; y: number } | null>(null);

  type Drag = {
    pointerId: number;
    kind: "move" | "resize" | "draw" | "line";
    handle: number;
    /** The frame and the document point when the drag began. */
    start: Bounds;
    from: [number, number];
    client: [number, number];
    moved: boolean;
  };
  let drag = $state<Drag | null>(null);

  // A ratio chosen fits the frame to it, a size makes the frame that size, both centered on the
  // frame as it is (and so when the tool opens).
  $effect(() => {
    // Read here, so that a value typed in the options bar fits the frame again.
    const ratio = aspectRatio(aspect);
    const size = aspect.mode === "size" ? [aspect.width, aspect.height] : null;
    untrack(() => {
      if (size && ratio) frame = centered(frame, Math.round(size[0]), Math.round(size[1]));
      else if (ratio) frame = fitRatio(frame, ratio);
    });
  });
  /** A size is set: the frame keeps it, it only moves. */
  const fixed = $derived(aspect.mode === "size" && aspectRatio(aspect) !== null);

  // Handles clockwise from the top-left corner (even: corners), and the edges each one moves.
  const MOVES_LEFT = [0, 6, 7];
  const MOVES_RIGHT = [2, 3, 4];
  const MOVES_TOP = [0, 1, 2];
  const MOVES_BOTTOM = [4, 5, 6];

  function handlePoints(b: Bounds): [number, number][] {
    const [cx, cy] = [(b.left + b.right) / 2, (b.top + b.bottom) / 2];
    return [
      [b.left, b.top],
      [cx, b.top],
      [b.right, b.top],
      [b.right, cy],
      [b.right, b.bottom],
      [cx, b.bottom],
      [b.left, b.bottom],
      [b.left, cy],
    ];
  }

  const screen = $derived(handlePoints(frame).map(([x, y]) => mapping.toViewport(x, y)));
  const [x0, y0] = $derived(mapping.toViewport(frame.left, frame.top));
  const [x1, y1] = $derived(mapping.toViewport(frame.right, frame.bottom));

  const CURSORS = [
    "nwse-resize",
    "ns-resize",
    "nesw-resize",
    "ew-resize",
    "nwse-resize",
    "ns-resize",
    "nesw-resize",
    "ew-resize",
  ];

  function begin(e: PointerEvent, kind: Drag["kind"], handle = 0) {
    if (e.button !== 0 || mapping.hand) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).closest("svg")?.setPointerCapture(e.pointerId);
    if (straighten) {
      const at = mapping.toDocument(e.clientX, e.clientY);
      line = { from: at, to: at };
      kind = "line";
    } else if (fixed && kind === "draw") {
      // A fixed size: the frame jumps under the pointer, then follows it.
      const [px, py] = mapping.toDocument(e.clientX, e.clientY);
      const [width, height] = [frame.right - frame.left, frame.bottom - frame.top];
      frame = centered({ left: px, top: py, right: px, bottom: py }, width, height);
      kind = "move";
    }
    drag = {
      pointerId: e.pointerId,
      kind,
      handle,
      start: { ...frame },
      from: mapping.toDocument(e.clientX, e.clientY),
      client: [e.clientX, e.clientY],
      moved: false,
    };
  }

  /** `b` with its edges in order and at least one pixel wide and high. */
  function normalized(b: Bounds): Bounds {
    const [left, right] = b.left <= b.right ? [b.left, b.right] : [b.right, b.left];
    const [top, bottom] = b.top <= b.bottom ? [b.top, b.bottom] : [b.bottom, b.top];
    return { left, top, right: Math.max(right, left + 1), bottom: Math.max(bottom, top + 1) };
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.moved) {
      if (Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]) < CLICK_SLOP) return;
      drag.moved = true;
    }
    const [px, py] = mapping.toDocument(e.clientX, e.clientY);
    if (drag.kind === "line") {
      if (line) line = { ...line, to: [px, py] };
      return;
    }
    const { start, from } = drag;
    const snaps = targets.length > 0 && !hasShortcutModifier(e);
    const threshold = SNAP_CSS_PX * mapping.docPerCss;
    let shown: SmartGuide[] = [];
    let next: Bounds;
    if (drag.kind === "move") {
      let [dx, dy] = [px - from[0], py - from[1]];
      if (snaps) {
        const snapped = snapMove(start, dx, dy, targets, threshold);
        [dx, dy] = [snapped.x, snapped.y];
        shown = snapped.guides;
      }
      [dx, dy] = [Math.round(dx), Math.round(dy)];
      next = {
        left: start.left + dx,
        top: start.top + dy,
        right: start.right + dx,
        bottom: start.bottom + dy,
      };
    } else {
      // The edges the drag moves follow the pointer (drawing: from where it began); the others
      // stay. Each moving edge snaps on its own axis.
      const drawing = drag.kind === "draw";
      const moves = (edges: number[]) => drawing || edges.includes(drag!.handle);
      const offset = (edge: number, at: number) => (drawing ? 0 : edge - at);
      let b: Bounds = drawing
        ? { left: from[0], top: from[1], right: px, bottom: py }
        : { ...start };
      if (!drawing) {
        if (moves(MOVES_LEFT)) b.left = px + offset(start.left, from[0]);
        if (moves(MOVES_RIGHT)) b.right = px + offset(start.right, from[0]);
        if (moves(MOVES_TOP)) b.top = py + offset(start.top, from[1]);
        if (moves(MOVES_BOTTOM)) b.bottom = py + offset(start.bottom, from[1]);
      }
      // The options bar's ratio, by every handle; else Shift on a corner (or drawing): the
      // proportions of the frame when the drag began.
      const corner = drawing || drag.handle % 2 === 0;
      const ratio =
        aspectRatio(aspect) ??
        (e.shiftKey && corner ? (start.right - start.left) / (start.bottom - start.top) : null);
      if (ratio) {
        b = keepRatio(b, start, drawing ? null : drag.handle, ratio);
      } else if (snaps) {
        const snapEdge = (edge: keyof Bounds, anchor: number, axis: "x" | "y") => {
          const snapped = snapHandle(b[edge], anchor, 1, axis, targets, threshold);
          if (!snapped) return null;
          b[edge] += snapped.shift;
          return snapped;
        };
        const used = [
          moves(MOVES_LEFT) && !drawing ? snapEdge("left", b.right, "x") : null,
          moves(MOVES_RIGHT) ? snapEdge("right", b.left, "x") : null,
          moves(MOVES_TOP) && !drawing ? snapEdge("top", b.bottom, "y") : null,
          moves(MOVES_BOTTOM) ? snapEdge("bottom", b.top, "y") : null,
        ];
        const placed = normalized(b);
        shown = used.flatMap((snap) => snap?.guides(placed) ?? []);
      }
      next = normalized({
        left: Math.round(b.left),
        top: Math.round(b.top),
        right: Math.round(b.right),
        bottom: Math.round(b.bottom),
      });
    }
    frame = next;
    guides = shown;
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    readout = {
      text: t("crop.readout", { width: next.right - next.left, height: next.bottom - next.top }),
      x: e.clientX - rect.left + 16,
      y: e.clientY - rect.top + 16,
    };
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    // A click outside the frame, as in Photoshop: apply.
    const apply = drag.kind === "draw" && !drag.moved;
    const drawn = drag.kind === "line" ? line : null;
    line = null;
    drag = null;
    if (drawn) {
      const turn = straightenTurn(drawn.from, drawn.to);
      if (turn !== null) onstraighten?.(turn);
      return;
    }
    readout = null;
    guides = [];
    if (apply) onapply(frame);
  }

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onKeydown(e: KeyboardEvent) {
    if (isTextField(e.target)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      onapply(frame);
    } else if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    }
  }

  const thirds = [1 / 3, 2 / 3];
</script>

<svelte:window onkeydown={onKeydown} />

<svg
  class="crop"
  role="presentation"
  style:cursor={straighten
    ? "crosshair"
    : drag?.kind === "resize"
      ? CURSORS[drag.handle]
      : "default"}
  onpointerdown={(e) => begin(e, "draw")}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
>
  <!-- The outside of the frame, shaded. -->
  <path
    class="shade"
    fill-rule="evenodd"
    d="M-10000 -10000H20000V20000H-10000Z M{x0} {y0}H{x1}V{y1}H{x0}Z"
  />
  <rect
    class="frame"
    x={x0}
    y={y0}
    width={Math.max(0, x1 - x0)}
    height={Math.max(0, y1 - y0)}
    role="presentation"
    onpointerdown={(e) => begin(e, "move")}
    ondblclick={() => onapply(frame)}
  />
  {#each thirds as f (f)}
    <line class="third" x1={x0 + (x1 - x0) * f} y1={y0} x2={x0 + (x1 - x0) * f} y2={y1} />
    <line class="third" x1={x0} y1={y0 + (y1 - y0) * f} x2={x1} y2={y0 + (y1 - y0) * f} />
  {/each}
  {#each fixed ? [] : screen as [x, y], i (i)}
    <rect
      class="handle"
      x={x - 4}
      y={y - 4}
      width="8"
      height="8"
      role="presentation"
      style:cursor={drag ? undefined : CURSORS[i]}
      onpointerdown={(e) => begin(e, "resize", i)}
    />
  {/each}
  {#if line}
    {@const [lx0, ly0] = mapping.toViewport(...line.from)}
    {@const [lx1, ly1] = mapping.toViewport(...line.to)}
    <line class="level" x1={lx0} y1={ly0} x2={lx1} y2={ly1} />
  {/if}
  {#if smartGuides}<SmartGuides {guides} {mapping} />{/if}
</svg>
{#if readout}
  <div class="readout" style:left="{readout.x}px" style:top="{readout.y}px">{readout.text}</div>
{/if}

<style>
  .crop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .shade {
    fill: rgba(0, 0, 0, 0.55);
    pointer-events: none;
  }

  .frame {
    fill: transparent;
    stroke: #fff;
    stroke-width: 1;
  }

  .third {
    stroke: rgba(255, 255, 255, 0.45);
    stroke-width: 1;
    pointer-events: none;
  }

  .level {
    stroke: #fff;
    stroke-width: 1.5;
    stroke-dasharray: 6 3;
    pointer-events: none;
  }

  .handle {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }

  .readout {
    position: absolute;
    padding: 3px 7px;
    border-radius: 4px;
    background: rgba(20, 20, 20, 0.85);
    color: #fff;
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
</style>

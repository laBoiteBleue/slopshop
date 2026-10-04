<script lang="ts">
  // Free Transform (Ctrl+T, as in Photoshop): a box around the selected layers with eight
  // handles and a reference point (the pivot, at the center until dragged). Drag inside to
  // move, a handle to scale (corners keep the proportions, Shift frees them; sides scale one
  // way, Shift keeps the proportions; Alt scales about the pivot), Ctrl and a side handle to
  // skew, and outside to rotate about the pivot (Shift: steps of 15°). The right-click menu
  // rotates and flips about the pivot. Enter, a double-click inside or a click outside (without
  // dragging) applies, Esc cancels. The owner applies `onchange`'s matrix live (ADR 0018).
  import * as affine from "./affine";
  import { isTextField } from "./keymap";
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { Bounds, Matrix } from "./engine";
  import { getLocale, t } from "./i18n/index.svelte";
  import type { ViewMapping } from "./Viewport.svelte";
  import SmartGuides from "./SmartGuides.svelte";
  import { SNAP_CSS_PX, type SmartGuide } from "./snap";
  import {
    boxFrame,
    movedBy,
    resizeCursor,
    rotatedTo,
    scalePercent,
    scaledTo,
    skewDegrees,
    skewedTo,
    snappedScale,
  } from "./freeTransform";
  import { hasShortcutModifier } from "./platform";

  let {
    mapping,
    box,
    matrix = $bindable(),
    pivot = $bindable(),
    targets = [],
    smartGuides = true,
    onchange,
    oncommit,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The selected layers' bounds when the transform began, in document pixels. */
    box: Bounds;
    /** Maps the box as it was to where it is now (the options bar's fields change it too). */
    matrix: Matrix;
    /** The reference point, in the box's coordinates: rotations and Alt scale about it. */
    pivot: [number, number];
    /**
     * What moves and scales snap to (the canvas, the other layers), as in Photoshop; none when
     * snapping is off. Ctrl held: no snapping.
     */
    targets?: Bounds[];
    /** The snaps' smart guides are drawn (View > Hide Extras hides them; the snap stays). */
    smartGuides?: boolean;
    /** The transform since the beginning, a map of the document's space. */
    onchange: (matrix: Matrix) => void;
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  type Drag = {
    pointerId: number;
    kind: "move" | "scale" | "rotate" | "skew" | "pivot";
    /** The handle dragged (scale). */
    handle: number;
    /** `matrix` and its angle when the drag began, and the document point it began at. */
    start: Matrix;
    startRotation: number;
    from: [number, number];
    /** Where the pointer went down (CSS pixels), and whether it has left that spot. */
    client: [number, number];
    moved: boolean;
  };
  /** A press moving less than this (CSS pixels) is a click, not a drag. */
  const CLICK_SLOP = 3;
  let drag = $state<Drag | null>(null);
  /** Smart guides of the current snap, in document pixels. */
  let guides = $state<SmartGuide[]>([]);
  /** What the drag does, shown next to the pointer. */
  let readout = $state<{ text: string; x: number; y: number } | null>(null);

  const frame = $derived(boxFrame(box, pivot));
  const center = $derived(frame.center);
  /** The right-click menu, where it opened. */
  let menuAt = $state<{ x: number; y: number } | null>(null);
  const handles = $derived(frame.handles);
  const screen = $derived(
    handles.map(([x, y]) => mapping.toViewport(...affine.apply(matrix, x, y))),
  );
  const screenCenter = $derived(mapping.toViewport(...affine.apply(matrix, ...center)));
  const screenPivot = $derived(mapping.toViewport(...affine.apply(matrix, ...pivot)));
  const outline = $derived([0, 2, 4, 6].map((i) => `${screen[i][0]},${screen[i][1]}`).join(" "));

  const RESIZE_CURSORS = ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"];
  /** Ctrl (⌘) is held: side handles skew, their arrows turn along the side. */
  let skewKey = $state(false);

  /**
   * A resize cursor along the direction from the center to handle `i`, as shown; for a side
   * handle that skews (Ctrl held, or a skew under way), a quarter turn: along the side it slides.
   */
  function cursorFor(i: number): string {
    const skews = i % 2 === 1 && (drag ? drag.kind === "skew" : skewKey);
    return RESIZE_CURSORS[resizeCursor(screen[i], screenCenter, skews)];
  }

  function begin(e: PointerEvent, kind: Drag["kind"], handle = 0) {
    // Other buttons, or Space held: the viewport pans.
    if (e.button !== 0 || mapping.hand) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).closest("svg")?.setPointerCapture(e.pointerId);
    drag = {
      pointerId: e.pointerId,
      kind,
      handle,
      start: matrix,
      // The box's angle, for Shift's steps (the fields may have changed it).
      startRotation: Math.atan2(matrix[1], matrix[0]),
      from: mapping.toDocument(e.clientX, e.clientY),
      client: [e.clientX, e.clientY],
      moved: false,
    };
  }

  /** Points the pivot snaps to (box coordinates): the handles and the center. */
  const pivotSnaps = $derived([...handles, center]);

  /** A rotation or flip of the box about the pivot (the right-click menu), as one step. */
  function turn(by: Matrix) {
    const [px, py] = affine.apply(matrix, ...pivot);
    matrix = affine.andThen(matrix, affine.about(by, px, py));
    onchange(matrix);
  }

  const menuItems = $derived.by((): MenuItem[] => {
    const command = (label: string, run: () => void): MenuItem => ({ kind: "command", label, run });
    return [
      command(t("menu.edit.transform.rotate180"), () => turn(affine.rotation(Math.PI))),
      command(t("menu.edit.transform.rotateCw"), () => turn(affine.rotation(Math.PI / 2))),
      command(t("menu.edit.transform.rotateCcw"), () => turn(affine.rotation(-Math.PI / 2))),
      { kind: "separator" },
      command(t("menu.edit.transform.flipHorizontal"), () => turn(affine.scaling(-1, 1))),
      command(t("menu.edit.transform.flipVertical"), () => turn(affine.scaling(1, -1))),
      { kind: "separator" },
      command(t("transform.apply"), oncommit),
      command(t("transform.cancel"), oncancel),
    ];
  });

  const number = (v: number, digits: number) =>
    new Intl.NumberFormat(getLocale(), {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(v);

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.moved) {
      const distance = Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]);
      if (distance < CLICK_SLOP) return;
      drag.moved = true;
    }
    const p = mapping.toDocument(e.clientX, e.clientY);
    const { start, from } = drag;
    const snapTo = hasShortcutModifier(e) ? [] : targets;
    const keys = { shift: e.shiftKey, alt: e.altKey };
    const threshold = SNAP_CSS_PX * mapping.docPerCss;
    let shown: SmartGuide[] = [];
    let text = "";
    if (drag.kind === "pivot") {
      const inverse = affine.invert(matrix);
      if (!inverse) return;
      let point = affine.apply(inverse, ...p);
      // Onto a handle or the center within the snap distance, as Photoshop's reference point.
      const [sx, sy] = mapping.toViewport(...p);
      for (const snap of pivotSnaps) {
        const [vx, vy] = mapping.toViewport(...affine.apply(matrix, ...snap));
        if (Math.hypot(vx - sx, vy - sy) <= SNAP_CSS_PX) point = snap;
      }
      pivot = point;
      guides = [];
      return;
    } else if (drag.kind === "skew") {
      matrix = skewedTo(frame, start, drag.handle, p, keys);
      text = t("transform.readout.skew", { angle: number(skewDegrees(matrix), 1) });
    } else if (drag.kind === "move") {
      const moved = movedBy(
        frame,
        start,
        p[0] - from[0],
        p[1] - from[1],
        e.shiftKey,
        snapTo,
        threshold,
      );
      shown = moved.guides;
      matrix = affine.andThen(start, affine.translation(moved.dx, moved.dy));
      text = t("transform.readout.move", { dx: number(moved.dx, 0), dy: number(moved.dy, 0) });
    } else if (drag.kind === "rotate") {
      const rotated = rotatedTo(frame, start, drag.startRotation, from, p, e.shiftKey);
      matrix = rotated.matrix;
      text = t("transform.readout.angle", { angle: number(rotated.degrees, 1) });
    } else {
      // The dragged handle snaps to the other layers' edges and centers, or to their sizes,
      // and the scale follows.
      const snapped = snappedScale(frame, start, drag.handle, p, keys, snapTo, threshold);
      matrix = snapped?.matrix ?? scaledTo(frame, start, drag.handle, p, keys);
      shown = snapped?.guides ?? [];
      // Relative to the size when the transform began.
      const scale = scalePercent(matrix);
      text = t("transform.readout.scale", {
        w: number(scale.width, 1),
        h: number(scale.height, 1),
      });
    }
    guides = shown;
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    readout = { text, x: e.clientX - rect.left + 16, y: e.clientY - rect.top + 16 };
    onchange(matrix);
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    // A click outside the box, as in Photoshop: apply.
    const commit = drag.kind === "rotate" && !drag.moved;
    drag = null;
    readout = null;
    guides = [];
    if (commit) oncommit();
  }

  function onKeydown(e: KeyboardEvent) {
    skewKey = hasShortcutModifier(e);
    if (isTextField(e.target)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      oncommit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    }
  }

  /** A curved arrow: rotate. */
  const ROTATE_CURSOR =
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Cpath d='M7 15a7 7 0 1 1 7 6' fill='none' stroke='black' stroke-width='4'/%3E%3Cpath d='M7 15a7 7 0 1 1 7 6' fill='none' stroke='white' stroke-width='2'/%3E%3Cpath d='M3 13h8l-4 5z' fill='white' stroke='black'/%3E%3C/svg%3E\") 12 12, crosshair";
</script>

<svelte:window
  onkeydown={onKeydown}
  onkeyup={(e) => (skewKey = hasShortcutModifier(e))}
  onblur={() => (skewKey = false)}
/>

<svg
  class="free-transform"
  role="presentation"
  style:cursor={drag?.kind === "scale" || drag?.kind === "skew"
    ? cursorFor(drag.handle)
    : drag?.kind === "move" || drag?.kind === "pivot"
      ? "default"
      : ROTATE_CURSOR}
  onpointerdown={(e) => begin(e, "rotate")}
  oncontextmenu={(e) => {
    e.preventDefault();
    menuAt = { x: e.clientX, y: e.clientY };
  }}
  onpointermove={(e) => {
    skewKey = hasShortcutModifier(e);
    onPointerMove(e);
  }}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
>
  <polygon
    class="body"
    points={outline}
    role="presentation"
    style:cursor={drag ? undefined : "default"}
    onpointerdown={(e) => begin(e, "move")}
    ondblclick={() => oncommit()}
  />
  {#each screen as [x, y], i (i)}
    <rect
      class="handle"
      x={x - 4}
      y={y - 4}
      width="8"
      height="8"
      role="presentation"
      style:cursor={drag ? undefined : cursorFor(i)}
      onpointerdown={(e) => begin(e, i % 2 === 1 && hasShortcutModifier(e) ? "skew" : "scale", i)}
    />
  {/each}
  <circle
    class="pivot"
    cx={screenPivot[0]}
    cy={screenPivot[1]}
    r="4"
    role="presentation"
    style:cursor={drag ? undefined : "move"}
    onpointerdown={(e) => begin(e, "pivot")}
  />
  {#if smartGuides}<SmartGuides {guides} {mapping} />{/if}
</svg>
{#if menuAt}
  <ContextMenu x={menuAt.x} y={menuAt.y} items={menuItems} onclose={() => (menuAt = null)} />
{/if}
{#if readout}
  <div class="readout" style:left="{readout.x}px" style:top="{readout.y}px">{readout.text}</div>
{/if}

<style>
  .free-transform {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .body {
    fill: transparent;
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .handle {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }

  .pivot {
    fill: transparent;
    stroke: #fff;
    stroke-width: 1.5;
    filter: drop-shadow(0 0 1px #000);
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

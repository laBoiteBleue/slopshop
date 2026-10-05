<script lang="ts">
  // Free Transform (Ctrl+T, as in Photoshop): a box around the selected layers with eight
  // handles and a reference point (the pivot, at the center until dragged). Drag inside to
  // move, a handle to scale (corners keep the proportions, Shift frees them; sides scale one
  // way, Shift keeps the proportions; Alt scales about the pivot), Ctrl and a side handle to
  // skew, and outside to rotate about the pivot (Shift: steps of 15°). The right-click menu
  // rotates and flips about the pivot. Enter, a double-click inside or a click outside (without
  // dragging) applies, Esc cancels. The owner applies `onchange`'s matrix live (ADR 0018).
  //
  // Distort and Perspective (ADR 0038, pixel layers only): Ctrl and a corner places it freely,
  // Alt+Shift+Ctrl and a corner moves the corner paired with it the other way (Photoshop's
  // Perspective); Edit > Transform > Distort and Perspective make that a corner's plain drag.
  // Once a corner is free, the box is a quad: corners distort, sides move with their two
  // corners, inside moves it, outside turns it; the map is the box to that quad.
  import * as affine from "./affine";
  import * as homography from "./homography";
  import type { Homography } from "./homography";
  import { isTextField } from "./keymap";
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { Bounds, Matrix } from "./engine";
  import { getLocale, t } from "./i18n/index.svelte";
  import type { ViewMapping } from "./Viewport.svelte";
  import SmartGuides from "./SmartGuides.svelte";
  import { SNAP_CSS_PX, type SmartGuide, type SnapTarget } from "./snap";
  import {
    boxFrame,
    movedBy,
    onWholePixels,
    resizeCursor,
    ROTATION_STEP,
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
    quad = $bindable(null),
    distortable = false,
    mode = $bindable("free"),
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
     * The box's corners (top left, top right, bottom right, bottom left) once one is placed
     * freely (Distort, Perspective): the map is then the box to this quad. Null while affine.
     */
    quad?: [number, number][] | null;
    /** Corners may be placed freely: pixel layers (the engine puts nothing else in perspective). */
    distortable?: boolean;
    /** Edit > Transform > Distort or Perspective: what a corner's plain drag does. */
    mode?: "free" | "distort" | "perspective";
    /**
     * What moves and scales snap to (the canvas, the other layers), as in Photoshop; none when
     * snapping is off. Ctrl held: no snapping.
     */
    targets?: SnapTarget[];
    /** The snaps' smart guides are drawn (View > Hide Extras hides them; the snap stays). */
    smartGuides?: boolean;
    /** The transform since the beginning, a map of the document's space (nine numbers once in perspective). */
    onchange: (matrix: Matrix | Homography) => void;
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  type Drag = {
    pointerId: number;
    kind: "move" | "scale" | "rotate" | "skew" | "pivot" | "distort" | "perspective" | "side";
    /** The box's corners in the document when the drag began. */
    quad: [number, number][];
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
  /** The map in perspective, once a corner is free. */
  const projective = $derived(quad ? homography.rectToQuad(box, quad) : null);
  /** A point of the box as it was, where it is now in the document. */
  function placed(x: number, y: number): [number, number] {
    return projective ? homography.apply(projective, x, y) : affine.apply(matrix, x, y);
  }
  /** The map so far: six numbers, or nine in perspective. */
  const current = (): Matrix | Homography => projective ?? matrix;
  const screen = $derived(handles.map(([x, y]) => mapping.toViewport(...placed(x, y))));
  const screenCenter = $derived(mapping.toViewport(...placed(...center)));
  const screenPivot = $derived(mapping.toViewport(...placed(...pivot)));
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
      quad: quad ?? homography.corners(box).map(([x, y]) => affine.apply(matrix, x, y)),
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
    const [px, py] = placed(...pivot);
    const about = affine.about(by, px, py);
    if (quad) {
      quad = quad.map(([x, y]) => affine.apply(about, x, y));
    } else {
      matrix = affine.andThen(matrix, about);
    }
    onchange(current());
  }

  /** What a press on handle `i` does: scale or skew, or with free corners distort them. */
  function handleKind(e: PointerEvent, i: number): Drag["kind"] {
    const ctrl = hasShortcutModifier(e);
    if (distortable && i % 2 === 0) {
      if (mode === "perspective" || (ctrl && e.altKey && e.shiftKey)) return "perspective";
      if (mode === "distort" || ctrl || quad) return "distort";
    }
    if (distortable && i % 2 === 1 && (mode !== "free" || quad)) return "side";
    return i % 2 === 1 && ctrl ? "skew" : "scale";
  }

  /** `next` as the box's corners, if a rectangle can be put in perspective to it. */
  function distortTo(next: [number, number][]) {
    if (homography.isConvex(next) && homography.rectToQuad(box, next)) quad = next;
  }

  const menuItems = $derived.by((): MenuItem[] => {
    const command = (label: string, run: () => void): MenuItem => ({ kind: "command", label, run });
    const modes: MenuItem[] = distortable
      ? [
          {
            kind: "command",
            label: t("menu.edit.transform.distort"),
            checked: mode === "distort",
            run: () => (mode = mode === "distort" ? "free" : "distort"),
          },
          {
            kind: "command",
            label: t("menu.edit.transform.perspective"),
            checked: mode === "perspective",
            run: () => (mode = mode === "perspective" ? "free" : "perspective"),
          },
          { kind: "separator" },
        ]
      : [];
    return [
      ...modes,
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
      const inverse = projective ? homography.invert(projective) : null;
      const affineInverse = projective ? null : affine.invert(matrix);
      if (!inverse && !affineInverse) return;
      let point = inverse ? homography.apply(inverse, ...p) : affine.apply(affineInverse!, ...p);
      // Onto a handle or the center within the snap distance, as Photoshop's reference point.
      const [sx, sy] = mapping.toViewport(...p);
      for (const snap of pivotSnaps) {
        const [vx, vy] = mapping.toViewport(...placed(...snap));
        if (Math.hypot(vx - sx, vy - sy) <= SNAP_CSS_PX) point = snap;
      }
      pivot = point;
      guides = [];
      return;
    } else if (drag.kind === "distort" || drag.kind === "perspective" || drag.kind === "side") {
      const [dx, dy] = [p[0] - from[0], p[1] - from[1]];
      const next = drag.quad.map(([x, y]) => [x, y] as [number, number]);
      if (drag.kind === "perspective") {
        distortTo(homography.perspectiveDrag(drag.quad, drag.handle / 2, dx, dy));
      } else {
        // A corner, or a side's two corners.
        const moved =
          drag.kind === "side"
            ? [(drag.handle - 1) / 2, ((drag.handle + 1) / 2) % 4]
            : [drag.handle / 2];
        for (const k of moved) next[k] = [next[k][0] + dx, next[k][1] + dy];
        distortTo(next);
      }
    } else if (quad && drag.kind === "move") {
      const [dx, dy] = [p[0] - from[0], p[1] - from[1]];
      quad = drag.quad.map(([x, y]) => [x + dx, y + dy]);
      text = t("transform.readout.move", { dx: number(dx, 0), dy: number(dy, 0) });
    } else if (quad && drag.kind === "rotate") {
      // About the pivot where it is now; Shift: steps of 15°.
      const [px, py] = placed(...pivot);
      let angle = Math.atan2(p[1] - py, p[0] - px) - Math.atan2(from[1] - py, from[0] - px);
      if (e.shiftKey) angle = Math.round(angle / ROTATION_STEP) * ROTATION_STEP;
      const about = affine.about(affine.rotation(angle), px, py);
      quad = drag.quad.map(([x, y]) => affine.apply(about, x, y));
      text = t("transform.readout.angle", { angle: number((angle * 180) / Math.PI, 1) });
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
      matrix = onWholePixels(box, affine.andThen(start, affine.translation(moved.dx, moved.dy)));
      text = t("transform.readout.move", { dx: number(moved.dx, 0), dy: number(moved.dy, 0) });
    } else if (drag.kind === "rotate") {
      const rotated = rotatedTo(frame, start, drag.startRotation, from, p, e.shiftKey);
      matrix = rotated.matrix;
      text = t("transform.readout.angle", { angle: number(rotated.degrees, 1) });
    } else {
      // The dragged handle snaps to the other layers' edges and centers, or to their sizes,
      // and the scale follows.
      const snapped = snappedScale(frame, start, drag.handle, p, keys, snapTo, threshold);
      matrix = onWholePixels(box, snapped?.matrix ?? scaledTo(frame, start, drag.handle, p, keys));
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
    readout = text ? { text, x: e.clientX - rect.left + 16, y: e.clientY - rect.top + 16 } : null;
    onchange(current());
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
    : drag?.kind === "rotate"
      ? ROTATE_CURSOR
      : drag
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
      onpointerdown={(e) => begin(e, handleKind(e, i), i)}
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

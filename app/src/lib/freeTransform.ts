// The geometry of Free Transform's gestures (FreeTransform.svelte), as in Photoshop: a box with
// eight handles and a reference point (the pivot); moves, scales (corners keep the proportions,
// Shift frees them; sides scale one way, Shift keeps the proportions; Alt about the pivot),
// skews and rotations (Shift: steps of 15°). Points are in document pixels; the box's own
// coordinates are the document's before the transform (`matrix` maps them to where they are).

import * as affine from "./affine.ts";
import type { Bounds, Matrix } from "./engine";
import { snapHandle, snapMove, type AxisSnap, type Guide } from "./snap.ts";

/** Rotation steps with Shift. */
export const ROTATION_STEP = Math.PI / 12;
/** Scales never reach 0 (the transform must stay invertible). */
export const MIN_SCALE = 1e-3;

type Point = [number, number];

/** The box's points, in its own coordinates. */
export type BoxFrame = {
  /** Clockwise from the top-left corner (even: corners, odd: sides). */
  handles: Point[];
  center: Point;
  /** The reference point: rotations and Alt scale about it. */
  pivot: Point;
};

/** The modifier keys a gesture reads. */
export type Keys = { shift: boolean; alt: boolean };

export function boxFrame(box: Bounds, pivot: Point): BoxFrame {
  const center: Point = [(box.left + box.right) / 2, (box.top + box.bottom) / 2];
  const [cx, cy] = center;
  const { left: l, top: t, right: r, bottom: b } = box;
  const handles: Point[] = [
    [l, t],
    [cx, t],
    [r, t],
    [r, cy],
    [r, b],
    [cx, b],
    [l, b],
    [l, cy],
  ];
  return { handles, center, pivot };
}

/** The box's bounds in the document under `m` (the box of its corners). */
export function boundsUnder(frame: BoxFrame, m: Matrix): Bounds {
  const corners = [0, 2, 4, 6].map((i) => affine.apply(m, ...frame.handles[i]));
  const xs = corners.map(([x]) => x);
  const ys = corners.map(([, y]) => y);
  return {
    left: Math.min(...xs),
    top: Math.min(...ys),
    right: Math.max(...xs),
    bottom: Math.max(...ys),
  };
}

/** The point a handle's drag scales or skews about: the opposite handle, or with Alt the pivot. */
function anchorOf(frame: BoxFrame, handle: number, keys: Keys): Point {
  return keys.alt ? frame.pivot : frame.handles[(handle + 4) % 8];
}

/** The scale by `handle` dragged to `p`, from `start`, as the keys say. */
export function scaledTo(
  frame: BoxFrame,
  start: Matrix,
  handle: number,
  p: Point,
  keys: Keys,
): Matrix {
  const inverse = affine.invert(start);
  if (!inverse) return start;
  const [qx, qy] = affine.apply(inverse, ...p);
  const [hx, hy] = frame.handles[handle];
  const [ax, ay] = anchorOf(frame, handle, keys);
  const [wx, wy] = [hx - ax, hy - ay];
  const ux = wx !== 0 ? (qx - ax) / wx : 1;
  const uy = wy !== 0 ? (qy - ay) / wy : 1;
  let [sx, sy] = [1, 1];
  if (handle % 2 === 0) {
    if (keys.shift) {
      [sx, sy] = [ux, uy];
    } else {
      // Along the diagonal: the pointer projected on it.
      const s = (wx * (qx - ax) + wy * (qy - ay)) / (wx * wx + wy * wy || 1);
      [sx, sy] = [s, s];
    }
  } else if (wy === 0) {
    sx = ux;
    if (keys.shift) sy = Math.abs(ux);
  } else {
    sy = uy;
    if (keys.shift) sx = Math.abs(uy);
  }
  const bounded = (s: number) => (Math.abs(s) < MIN_SCALE ? Math.sign(s || 1) * MIN_SCALE : s);
  return affine.then(affine.about(affine.scaling(bounded(sx), bounded(sy)), ax, ay), start);
}

/**
 * The skew by side `handle` dragged to `p`, from `start`: the side slides along itself, the
 * opposite one stays (Alt: about the pivot).
 */
export function skewedTo(
  frame: BoxFrame,
  start: Matrix,
  handle: number,
  p: Point,
  keys: Keys,
): Matrix {
  const inverse = affine.invert(start);
  if (!inverse) return start;
  const [qx, qy] = affine.apply(inverse, ...p);
  const [hx, hy] = frame.handles[handle];
  const [ax, ay] = anchorOf(frame, handle, keys);
  // Top and bottom sides slide horizontally, left and right ones vertically.
  const horizontal = hy !== frame.center[1];
  const by: Matrix = horizontal
    ? [1, 0, hy !== ay ? (qx - hx) / (hy - ay) : 0, 1, 0, 0]
    : [1, hx !== ax ? (qy - hy) / (hx - ax) : 0, 0, 1, 0, 0];
  return affine.then(affine.about(by, ax, ay), start);
}

/**
 * The rotation about the pivot (as placed by `start`) that turns `from` towards `p`; with Shift
 * the box's angle (`startRotation` plus the turn) goes by steps. Also the angle in degrees, in
 * (-180°, 180°] as Photoshop shows it.
 */
export function rotatedTo(
  frame: BoxFrame,
  start: Matrix,
  startRotation: number,
  from: Point,
  p: Point,
  shift: boolean,
): { matrix: Matrix; degrees: number } {
  const [cx, cy] = affine.apply(start, ...frame.pivot);
  const turned = Math.atan2(p[1] - cy, p[0] - cx) - Math.atan2(from[1] - cy, from[0] - cx);
  let total = startRotation + turned;
  if (shift) total = Math.round(total / ROTATION_STEP) * ROTATION_STEP;
  const matrix = affine.then(start, affine.about(affine.rotation(total - startRotation), cx, cy));
  let degrees = ((((total * 180) / Math.PI) % 360) + 360) % 360;
  if (degrees > 180) degrees -= 360;
  return { matrix, degrees };
}

/**
 * The move by (`dx`, `dy`) from `start`: with Shift along the larger axis only; snapped to
 * `targets` within `threshold` when there are some (the locked axis stays put).
 */
export function movedBy(
  frame: BoxFrame,
  start: Matrix,
  dx: number,
  dy: number,
  shift: boolean,
  targets: Bounds[],
  threshold: number,
): { dx: number; dy: number; guides: Guide[] } {
  if (shift) {
    if (Math.abs(dx) >= Math.abs(dy)) dy = 0;
    else dx = 0;
  }
  if (targets.length === 0) return { dx, dy, guides: [] };
  const snapped = snapMove(boundsUnder(frame, start), dx, dy, targets, threshold);
  return {
    dx: !shift || dx !== 0 ? snapped.x : dx,
    dy: !shift || dy !== 0 ? snapped.y : dy,
    guides: snapped.guides,
  };
}

/**
 * The scale by `handle` dragged to `p` with its handle snapped to `targets` (their edges,
 * centers or sizes, as in Photoshop), and the guides; null when nothing is near or the box is
 * rotated (it only snaps upright).
 */
export function snappedScale(
  frame: BoxFrame,
  start: Matrix,
  handle: number,
  p: Point,
  keys: Keys,
  targets: Bounds[],
  threshold: number,
): { matrix: Matrix; guides: Guide[] } | null {
  const unrotated = Math.abs(start[1]) < 1e-9 && Math.abs(start[2]) < 1e-9;
  if (targets.length === 0 || !unrotated) return null;
  const scaled = scaledTo(frame, start, handle, p, keys);
  const [hx, hy] = affine.apply(scaled, ...frame.handles[handle]);
  const [ax, ay] = affine.apply(start, ...anchorOf(frame, handle, keys));
  const span = keys.alt ? 2 : 1;
  const side = handle % 2 === 1;
  const horizontal = side && frame.handles[handle][1] === frame.center[1];
  const onX = !side || horizontal ? snapHandle(hx, ax, span, "x", targets, threshold) : null;
  const onY = !side || !horizontal ? snapHandle(hy, ay, span, "y", targets, threshold) : null;
  let moved: Point | null = null;
  let used: (AxisSnap | null)[] = [];
  if (!side && !keys.shift) {
    // Proportional: one scale for both axes, set by the axis that needs the least shift.
    const x = onX && hx !== ax ? { snap: onX, ratio: (hx + onX.shift - ax) / (hx - ax) } : null;
    const y = onY && hy !== ay ? { snap: onY, ratio: (hy + onY.shift - ay) / (hy - ay) } : null;
    const pick = x && y ? (Math.abs(x.snap.shift) <= Math.abs(y.snap.shift) ? x : y) : (x ?? y);
    if (pick) {
      moved = [ax + (hx - ax) * pick.ratio, ay + (hy - ay) * pick.ratio];
      used = [pick.snap];
    }
  } else if (onX || onY) {
    moved = [p[0] + (onX?.shift ?? 0), p[1] + (onY?.shift ?? 0)];
    used = [onX, onY];
  }
  if (!moved) return null;
  const matrix = scaledTo(frame, start, handle, moved, keys);
  const placed = boundsUnder(frame, matrix);
  return { matrix, guides: used.flatMap((snap) => snap?.guides(placed) ?? []) };
}

/** The width and height under `m`, in percent of the box's (along its own axes). */
export function scalePercent(m: Matrix): { width: number; height: number } {
  const [a, b, c, d] = m;
  return {
    width: Math.hypot(a, b) * 100,
    height: (Math.abs(a * d - b * c) / Math.hypot(a, b)) * 100,
  };
}

/** The angle the sides lean by under `m`, from upright, in degrees. */
export function skewDegrees(m: Matrix): number {
  const [a, b, c, d] = m;
  return (Math.atan2(c * a + d * b, a * d - b * c) * 180) / Math.PI;
}

/**
 * Which of the four resize cursors (0: ↔, 1: ↘, 2: ↕, 3: ↗) points from the box's center to a
 * handle, as shown on screen; a skewing side handle gets a quarter turn (along its side).
 */
export function resizeCursor(handle: Point, center: Point, skews: boolean): number {
  const angle = (Math.atan2(handle[1] - center[1], handle[0] - center[0]) * 180) / Math.PI;
  const step = Math.round((((angle % 180) + 180) % 180) / 45) % 4;
  return skews ? (step + 2) % 4 : step;
}

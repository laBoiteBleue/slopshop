// Snapping of on-canvas gestures (the Move tool, Free Transform, Crop, a guide dragged) to the
// canvas, the other layers and the guides, as in Photoshop: edges and centers meet within a few
// screen pixels, and magenta smart guides show each alignment. One engine for every gesture.
// Gesture geometry only, in document pixels.

import type { Bounds, Guide } from "./engine";

/**
 * A smart guide: a line in document pixels. A `measure` (an equal size) ends with short
 * perpendicular ticks, like the serifs of an I.
 */
export type SmartGuide = { x1: number; y1: number; x2: number; y2: number; measure?: boolean };

/** What a gesture snaps to: a box (the canvas, a layer: its edges and center) or a guide. */
export type SnapTarget = Bounds | Guide;

/** Snapping distance, in screen (CSS) pixels. */
export const SNAP_CSS_PX = 6;

/**
 * A line to snap to (x of a vertical one, y of a horizontal one), and its target's extent
 * across (`null`: a guide, across the whole document).
 */
type Line = { at: number; span: [number, number] | null };

const isGuide = (t: SnapTarget): t is Guide => "position" in t;

function verticals(targets: SnapTarget[]): Line[] {
  return targets.flatMap((t): Line[] => {
    if (isGuide(t)) return t.vertical ? [{ at: t.position, span: null }] : [];
    return [t.left, (t.left + t.right) / 2, t.right].map((at) => ({ at, span: [t.top, t.bottom] }));
  });
}

function horizontals(targets: SnapTarget[]): Line[] {
  return targets.flatMap((t): Line[] => {
    if (isGuide(t)) return t.vertical ? [] : [{ at: t.position, span: null }];
    return [t.top, (t.top + t.bottom) / 2, t.bottom].map((at) => ({ at, span: [t.left, t.right] }));
  });
}

/** The line nearest to one of `edges` within `threshold`, and the shift that reaches it. */
function nearest(
  edges: number[],
  lines: Line[],
  threshold: number,
): { shift: number; line: Line } | null {
  let found: { shift: number; line: Line } | null = null;
  for (const edge of edges) {
    for (const line of lines) {
      const shift = line.at - edge;
      if (Math.abs(shift) <= threshold && (!found || Math.abs(shift) < Math.abs(found.shift))) {
        found = { shift, line };
      }
    }
  }
  return found;
}

/**
 * Smart guides for alignments on `x` and `y`, spanning `box` and the targets aligned with (a
 * guide's alignment spans the box alone: the guide is drawn already).
 */
function guidesFor(box: Bounds, x: Line | null, y: Line | null): SmartGuide[] {
  const guides: SmartGuide[] = [];
  if (x) {
    const [top, bottom] = x.span ?? [box.top, box.bottom];
    guides.push({
      x1: x.at,
      x2: x.at,
      y1: Math.min(box.top, top),
      y2: Math.max(box.bottom, bottom),
    });
  }
  if (y) {
    const [left, right] = y.span ?? [box.left, box.right];
    guides.push({
      y1: y.at,
      y2: y.at,
      x1: Math.min(box.left, left),
      x2: Math.max(box.right, right),
    });
  }
  return guides;
}
/**
 * The move (`x`, `y`) of `box` adjusted so that an edge or the center of the box meets an edge
 * or the center of one of `targets` within `threshold`, per axis, with a guide for each.
 */
export function snapMove(
  box: Bounds,
  x: number,
  y: number,
  targets: SnapTarget[],
  threshold: number,
): { x: number; y: number; guides: SmartGuide[] } {
  const moved = {
    left: box.left + x,
    top: box.top + y,
    right: box.right + x,
    bottom: box.bottom + y,
  };
  const onX = nearest(
    [moved.left, (moved.left + moved.right) / 2, moved.right],
    verticals(targets),
    threshold,
  );
  const onY = nearest(
    [moved.top, (moved.top + moved.bottom) / 2, moved.bottom],
    horizontals(targets),
    threshold,
  );
  const nx = x + (onX?.shift ?? 0);
  const ny = y + (onY?.shift ?? 0);
  const placed = {
    left: box.left + nx,
    top: box.top + ny,
    right: box.right + nx,
    bottom: box.bottom + ny,
  };
  return { x: nx, y: ny, guides: guidesFor(placed, onX?.line ?? null, onY?.line ?? null) };
}

/** How a dragged handle snaps on one axis: the shift to apply, and the guides to draw. */
export type AxisSnap = { shift: number; guides: (box: Bounds) => SmartGuide[] };

/**
 * Where a handle dragged along `axis` snaps (Free Transform's scaling, as in Photoshop): onto
 * an edge or a center of one of `targets`, or where the box gets the same size as one of them
 * along that axis; the smaller shift wins, within `threshold`. `handle` and `anchor` are the
 * handle's and the fixed point's positions on the axis; the box's size along it is `span` ×
 * their distance (1: the opposite side is fixed, 2: the center is).
 */
export function snapHandle(
  handle: number,
  anchor: number,
  span: number,
  axis: "x" | "y",
  targets: SnapTarget[],
  threshold: number,
): AxisSnap | null {
  const vertical = axis === "x";
  let found: AxisSnap | null = null;
  const consider = (shift: number, guides: AxisSnap["guides"]) => {
    if (Math.abs(shift) <= threshold && (!found || Math.abs(shift) < Math.abs(found.shift))) {
      found = { shift, guides };
    }
  };
  const lines = nearest([handle], vertical ? verticals(targets) : horizontals(targets), threshold);
  if (lines) {
    const line = lines.line;
    consider(lines.shift, (box) => guidesFor(box, vertical ? line : null, vertical ? null : line));
  }
  const direction = Math.sign(handle - anchor) || 1;
  for (const t of targets) {
    if (isGuide(t)) continue;
    const size = vertical ? t.right - t.left : t.bottom - t.top;
    if (size <= 0) continue;
    const shift = anchor + (direction * size) / span - handle;
    // The same size: a measure across the middle of the box and of the target.
    const middle = (lo: number, hi: number) => (lo + hi) / 2;
    const across = (b: Bounds): SmartGuide =>
      vertical
        ? { x1: b.left, x2: b.right, y1: middle(b.top, b.bottom), y2: middle(b.top, b.bottom) }
        : { x1: middle(b.left, b.right), x2: middle(b.left, b.right), y1: b.top, y2: b.bottom };
    consider(shift, (box) => [
      { ...across(box), measure: true },
      { ...across(t), measure: true },
    ]);
  }
  return found;
}

/**
 * Where a guide dragged to `at` lands: onto an edge or the center of one of `targets` on its
 * axis within `threshold`, else where it is.
 */
export function snapGuide(
  at: number,
  vertical: boolean,
  targets: SnapTarget[],
  threshold: number,
): number {
  const found = nearest([at], vertical ? verticals(targets) : horizontals(targets), threshold);
  return at + (found?.shift ?? 0);
}

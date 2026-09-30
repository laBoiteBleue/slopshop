// Snapping of on-canvas gestures (the Move tool, Free Transform) to the canvas and the other
// layers, as in Photoshop: edges and centers meet within a few screen pixels, and magenta smart
// guides show each alignment. Gesture geometry only, in document pixels.

import type { Bounds } from "./engine";

/** A smart guide: a line in document pixels. */
export type Guide = { x1: number; y1: number; x2: number; y2: number };

/** Snapping distance, in screen (CSS) pixels. */
export const SNAP_CSS_PX = 6;

/** A line to snap to (x of a vertical one, y of a horizontal one), and its target's extent. */
type Line = { at: number; span: [number, number] };

function verticals(targets: Bounds[]): Line[] {
  return targets.flatMap((b) =>
    [b.left, (b.left + b.right) / 2, b.right].map((at) => ({
      at,
      span: [b.top, b.bottom] as [number, number],
    })),
  );
}

function horizontals(targets: Bounds[]): Line[] {
  return targets.flatMap((b) =>
    [b.top, (b.top + b.bottom) / 2, b.bottom].map((at) => ({
      at,
      span: [b.left, b.right] as [number, number],
    })),
  );
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

/** Guides for alignments on `x` and `y`, spanning `box` and the targets aligned with. */
function guidesFor(box: Bounds, x: Line | null, y: Line | null): Guide[] {
  const guides: Guide[] = [];
  if (x) {
    guides.push({
      x1: x.at,
      x2: x.at,
      y1: Math.min(box.top, x.span[0]),
      y2: Math.max(box.bottom, x.span[1]),
    });
  }
  if (y) {
    guides.push({
      y1: y.at,
      y2: y.at,
      x1: Math.min(box.left, y.span[0]),
      x2: Math.max(box.right, y.span[1]),
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
  targets: Bounds[],
  threshold: number,
): { x: number; y: number; guides: Guide[] } {
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
export type AxisSnap = { shift: number; guides: (box: Bounds) => Guide[] };

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
  targets: Bounds[],
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
    const size = vertical ? t.right - t.left : t.bottom - t.top;
    if (size <= 0) continue;
    const shift = anchor + (direction * size) / span - handle;
    // The same size: a measure across the middle of the box and of the target.
    const middle = (lo: number, hi: number) => (lo + hi) / 2;
    consider(shift, (box) =>
      vertical
        ? [
            {
              x1: box.left,
              x2: box.right,
              y1: middle(box.top, box.bottom),
              y2: middle(box.top, box.bottom),
            },
            { x1: t.left, x2: t.right, y1: middle(t.top, t.bottom), y2: middle(t.top, t.bottom) },
          ]
        : [
            {
              x1: middle(box.left, box.right),
              x2: middle(box.left, box.right),
              y1: box.top,
              y2: box.bottom,
            },
            { x1: middle(t.left, t.right), x2: middle(t.left, t.right), y1: t.top, y2: t.bottom },
          ],
    );
  }
  return found;
}

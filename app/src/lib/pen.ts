// The Pen tool (P, ADR 0041): a path built anchor by anchor. A click adds a corner anchor, a
// drag a smooth one whose handles stay opposite (Alt during the drag frees the outgoing one),
// Shift keeps angles at 45° steps, a click on the first anchor closes the path. What the path
// is, what the next click or drag would add, the geometry sent to the engine and the outline
// shown while drawing.
import { snapped45 } from "./gradient";
import type { ShapeGeometry } from "./shapes";

export type Point = [number, number];

/** An anchor and its two handles (absolute, document pixels); a corner's handles are on it. */
export type Anchor = {
  point: Point;
  /** The handle the segment arriving here bends toward. */
  in: Point;
  /** The handle the segment leaving here bends toward. */
  out: Point;
};

/** A path being drawn. */
export type PenPath = { anchors: Anchor[]; closed: boolean };

/** A path's geometry as the engine takes it (`crate::shape::GeometryDto::Path`). */
export type PathGeometry = Extract<ShapeGeometry, { kind: "path" }>;

export function corner(point: Point): Anchor {
  return { point, in: point, out: point };
}

function same(a: Point, b: Point): boolean {
  return a[0] === b[0] && a[1] === b[1];
}

function mirror(point: Point, handle: Point): Point {
  return [2 * point[0] - handle[0], 2 * point[1] - handle[1]];
}

/**
 * Where a click at `at` puts the next anchor: there, or with `shift` on the nearest 45° step
 * from the last anchor (as Photoshop's Pen).
 */
export function nextPoint(path: PenPath | null, at: Point, shift: boolean): Point {
  const last = path?.anchors.at(-1);
  return last ? snapped45(last.point, at, shift) : at;
}

/**
 * The anchor at `point` dragged to `to`: its outgoing handle at `to` (on a 45° step from the
 * anchor with `shift`), the incoming one opposite, or left at `frozenIn` once Alt freed them.
 */
export function dragged(point: Point, to: Point, shift: boolean, frozenIn: Point | null): Anchor {
  const out = snapped45(point, to, shift);
  return { point, in: frozenIn ?? mirror(point, out), out };
}

/** A segment from `a` to `b`: straight when neither bends it, else a cubic Bézier. */
function segment(a: Anchor, b: Anchor): number[] {
  if (same(a.out, a.point) && same(b.in, b.point)) return [...b.point];
  return [...a.out, ...b.in, ...b.point];
}

/** `path` with `anchor` added at its end. */
export function withAnchor(path: PenPath | null, anchor: Anchor): PenPath {
  return { anchors: [...(path?.anchors ?? []), anchor], closed: false };
}

/** `path` without its last anchor; `null` when none is left (Backspace). */
export function withoutLast(path: PenPath): PenPath | null {
  return path.anchors.length > 1 ? { anchors: path.anchors.slice(0, -1), closed: false } : null;
}

/** `path` closed on its first anchor, that anchor's handles replaced by `first` if given. */
export function closed(path: PenPath, first?: Anchor): PenPath {
  const anchors = [...path.anchors];
  if (first) anchors[0] = first;
  return { anchors, closed: true };
}

/** Whether `path` is worth a layer: two anchors at least (an open path), three to close. */
export function isDrawable(path: PenPath | null): path is PenPath {
  return path !== null && path.anchors.length >= 2;
}

/** The engine's geometry of `path` (one subpath, nonzero: Photoshop's default for shapes). */
export function geometryOf(path: PenPath): PathGeometry {
  const [first, ...rest] = path.anchors;
  const segments: number[][] = [];
  let previous = first;
  for (const anchor of rest) {
    segments.push(segment(previous, anchor));
    previous = anchor;
  }
  if (path.closed && path.anchors.length > 1) segments.push(segment(previous, first));
  return {
    kind: "path",
    rule: "nonzero",
    subpaths: [{ start: first.point, segments, closed: path.closed }],
  };
}

/**
 * An SVG path of `geometry`, each point mapped by `map` (the viewport's mapping is affine, so
 * control points map exactly).
 */
export function svgOfPath(geometry: PathGeometry, map: (x: number, y: number) => Point): string {
  const pt = (x: number, y: number) => {
    const [vx, vy] = map(x, y);
    return `${vx.toFixed(2)} ${vy.toFixed(2)}`;
  };
  return geometry.subpaths
    .map((subpath) => {
      const parts = [`M${pt(...subpath.start)}`];
      for (const s of subpath.segments) {
        parts.push(
          s.length === 2
            ? `L${pt(s[0], s[1])}`
            : `C${pt(s[0], s[1])} ${pt(s[2], s[3])} ${pt(s[4], s[5])}`,
        );
      }
      if (subpath.closed) parts.push("Z");
      return parts.join(" ");
    })
    .join(" ");
}

/**
 * The rubber band: the segment the next click at `at` would add, from the last anchor (bent by
 * its outgoing handle), `null` before the first anchor.
 */
export function rubberBand(path: PenPath | null, at: Point): PathGeometry | null {
  const last = path?.anchors.at(-1);
  if (!last) return null;
  return {
    kind: "path",
    rule: "nonzero",
    subpaths: [{ start: last.point, segments: [segment(last, corner(at))], closed: false }],
  };
}

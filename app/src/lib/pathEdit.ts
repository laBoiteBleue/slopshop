// Editing a vector layer's path with the Pen (ADR 0041), as Photoshop's Pen and Direct
// Selection do: a path's subpaths as anchors and handles, what a press lands on, and the edits
// (moving an anchor, a handle or the whole path, adding an anchor on a segment, deleting one,
// converting one between corner and smooth, continuing an open path from an end).
import * as affine from "./affine";
import type { LayerView, Matrix } from "./engine";
import { findLayer, layerTree, ancestors } from "./layerTree";
import {
  corner,
  geometryOf,
  type Anchor,
  type PathGeometry,
  type PenPath,
  type Point,
} from "./pen";

/** The active vector layer, its path editable with the Pen. */
export type EditablePath = {
  layerId: number;
  /** In the layer's space: its path, or a live shape's as a path (`livePath`). */
  geometry: PathGeometry;
  /** A live shape: turned into a path before its anchors change. */
  live: boolean;
  /** From the layer's space to the document's; null in perspective (not editable). */
  matrix: Matrix | null;
  /** The layer's vector mask (ADR 0041) rather than its shape. Absent: its shape. */
  vectorMask?: boolean;
  /** A path of the document (the Paths panel's) rather than a layer's: `layerId` unused. */
  pathId?: number;
};

/** What the Pen edits of `layer` (a vector layer among `layers`), null for other layers. */
export function editablePath(layers: LayerView[], layer: LayerView | null): EditablePath | null {
  if (layer?.kind !== "vector" || !layer.shape) return null;
  const own = layer.shape.geometry;
  const geometry = own.kind === "path" ? own : layer.livePath;
  if (!geometry || geometry.kind !== "path") return null;
  return {
    layerId: layer.id,
    geometry,
    live: own.kind !== "path",
    matrix: layerToDocument(layers, layer.id),
  };
}

function same(a: Point, b: Point): boolean {
  return a[0] === b[0] && a[1] === b[1];
}

/**
 * The subpaths of `geometry` as anchors. A closed subpath whose last segment ends on its start
 * (as the Pen draws one) gives that start one anchor, its incoming handle from that segment;
 * otherwise its closing segment is straight.
 */
export function pathsOf(geometry: PathGeometry): PenPath[] {
  return geometry.subpaths.map((subpath) => {
    const anchors: Anchor[] = [corner(subpath.start)];
    for (const s of subpath.segments) {
      const previous = anchors[anchors.length - 1];
      if (s.length === 6) {
        previous.out = [s[0], s[1]];
        anchors.push({ point: [s[4], s[5]], in: [s[2], s[3]], out: [s[4], s[5]] });
      } else {
        anchors.push(corner([s[0], s[1]]));
      }
    }
    const last = anchors[anchors.length - 1];
    if (subpath.closed && anchors.length > 1 && same(last.point, anchors[0].point)) {
      anchors[0].in = last.in;
      anchors.pop();
    }
    return { anchors, closed: subpath.closed };
  });
}

/** The engine's geometry of `paths`, filled by `rule`. */
export function geometryOfPaths(paths: PenPath[], rule: PathGeometry["rule"]): PathGeometry {
  return { kind: "path", rule, subpaths: paths.map((p) => geometryOf(p).subpaths[0]) };
}

/** `path` with every point mapped by `map`. */
export function mappedPath(path: PenPath, map: (p: Point) => Point): PenPath {
  return {
    anchors: path.anchors.map((a) => ({ point: map(a.point), in: map(a.in), out: map(a.out) })),
    closed: path.closed,
  };
}

/**
 * From layer `id`'s space to the document's: its transform and its groups', composed; null when
 * one of them is in perspective (ADR 0038), its handles then not mapped by an affine map.
 */
export function layerToDocument(layers: LayerView[], id: number): Matrix | null {
  const tree = layerTree(layers);
  let m = affine.IDENTITY;
  for (const at of [id, ...ancestors(tree, id)]) {
    const layer = findLayer(layers, at);
    if (!layer || layer.perspective) return null;
    m = affine.andThen(m, layer.transform);
  }
  return m;
}

/** A segment's anchors: segment `i` leaves anchor `i` (the last one closes a closed path). */
function ends(path: PenPath, i: number): [Anchor, Anchor] {
  return [path.anchors[i], path.anchors[(i + 1) % path.anchors.length]];
}

/** How many segments `path` has. */
function segmentCount(path: PenPath): number {
  const n = path.anchors.length;
  return path.closed ? n : n - 1;
}

function lerp(a: Point, b: Point, t: number): Point {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
}

/** Whether the segment from `a` to `b` is a line (neither bends it). */
function isStraight(a: Anchor, b: Anchor): boolean {
  return same(a.out, a.point) && same(b.in, b.point);
}

/** The point at `t` of the segment from `a` to `b` (a line's evenly). */
function pointAt(a: Anchor, b: Anchor, t: number): Point {
  if (isStraight(a, b)) return lerp(a.point, b.point, t);
  const ab = lerp(a.point, a.out, t);
  const bc = lerp(a.out, b.in, t);
  const cd = lerp(b.in, b.point, t);
  const abc = lerp(ab, bc, t);
  const bcd = lerp(bc, cd, t);
  return lerp(abc, bcd, t);
}

/** What a press lands on. */
export type Hit =
  | { kind: "anchor"; path: number; index: number }
  | { kind: "handle"; path: number; index: number; which: "in" | "out" }
  | { kind: "segment"; path: number; index: number; t: number };

/** Samples per segment when finding the nearest point of a curve. */
const SAMPLES = 32;

/**
 * What a press at `at` (viewport pixels) lands on, within `radius`: an anchor first, then a
 * handle (with `handles`), then a segment. `toView` maps document pixels to the viewport.
 */
export function hitTest(
  paths: PenPath[],
  at: Point,
  radius: number,
  toView: (p: Point) => Point,
  handles: boolean,
): Hit | null {
  const near = (p: Point) => {
    const [x, y] = toView(p);
    return Math.hypot(x - at[0], y - at[1]) <= radius;
  };
  for (const [pi, path] of paths.entries()) {
    const i = path.anchors.findIndex((a) => near(a.point));
    if (i >= 0) return { kind: "anchor", path: pi, index: i };
  }
  if (handles) {
    for (const [pi, path] of paths.entries()) {
      for (const [i, a] of path.anchors.entries()) {
        for (const which of ["in", "out"] as const) {
          if (!same(a[which], a.point) && near(a[which])) {
            return { kind: "handle", path: pi, index: i, which };
          }
        }
      }
    }
  }
  let best: { hit: Hit; distance: number } | null = null;
  for (const [pi, path] of paths.entries()) {
    for (let i = 0; i < segmentCount(path); i++) {
      const [a, b] = ends(path, i);
      let previous = toView(a.point);
      for (let k = 1; k <= SAMPLES; k++) {
        const next = toView(pointAt(a, b, k / SAMPLES));
        // The nearest point of this piece of the curve, taken as a chord.
        const [dx, dy] = [next[0] - previous[0], next[1] - previous[1]];
        const length2 = dx * dx + dy * dy;
        const u =
          length2 > 0
            ? Math.min(
                1,
                Math.max(0, ((at[0] - previous[0]) * dx + (at[1] - previous[1]) * dy) / length2),
              )
            : 0;
        const distance = Math.hypot(previous[0] + u * dx - at[0], previous[1] + u * dy - at[1]);
        if (distance <= radius && (!best || distance < best.distance)) {
          best = {
            hit: { kind: "segment", path: pi, index: i, t: (k - 1 + u) / SAMPLES },
            distance,
          };
        }
        previous = next;
      }
    }
  }
  return best?.hit ?? null;
}

/** `paths` with path `i` replaced by `path` (removed when null). */
export function replaced(paths: PenPath[], i: number, path: PenPath | null): PenPath[] {
  return paths.flatMap((p, k) => (k !== i ? [p] : path ? [path] : []));
}

/** Anchor `i` replaced by `anchor` (Alt-drag: new smooth handles). */
export function withAnchorAt(path: PenPath, i: number, anchor: Anchor): PenPath {
  return { ...path, anchors: path.anchors.map((a, k) => (k === i ? anchor : a)) };
}

function shifted(p: Point, dx: number, dy: number): Point {
  return [p[0] + dx, p[1] + dy];
}

/** Anchor `i` of `path` moved by `(dx, dy)`, its handles with it. */
export function movedAnchor(path: PenPath, i: number, dx: number, dy: number): PenPath {
  const a = path.anchors[i];
  return withAnchorAt(path, i, {
    point: shifted(a.point, dx, dy),
    in: shifted(a.in, dx, dy),
    out: shifted(a.out, dx, dy),
  });
}

/** `path` moved by `(dx, dy)`. */
export function movedPath(path: PenPath, dx: number, dy: number): PenPath {
  return mappedPath(path, (p) => shifted(p, dx, dy));
}

/** Whether `a`'s handles are both out and opposite (a smooth anchor). */
export function isSmooth(a: Anchor): boolean {
  if (same(a.in, a.point) || same(a.out, a.point)) return false;
  const [ix, iy] = [a.in[0] - a.point[0], a.in[1] - a.point[1]];
  const [ox, oy] = [a.out[0] - a.point[0], a.out[1] - a.point[1]];
  const cross = ix * oy - iy * ox;
  const dot = ix * ox + iy * oy;
  return dot < 0 && Math.abs(cross) <= 1e-3 * Math.hypot(ix, iy) * Math.hypot(ox, oy);
}

/**
 * Handle `which` of anchor `i` moved to `to`. A smooth anchor stays smooth (the other handle
 * turns with it, keeping its length) unless `independent` (Alt, Photoshop's Convert Point on a
 * handle).
 */
export function movedHandle(
  path: PenPath,
  i: number,
  which: "in" | "out",
  to: Point,
  independent: boolean,
): PenPath {
  const a = path.anchors[i];
  const other = which === "in" ? "out" : "in";
  const moved: Anchor = { ...a, [which]: to };
  if (!independent && isSmooth(a)) {
    const [dx, dy] = [to[0] - a.point[0], to[1] - a.point[1]];
    const length = Math.hypot(dx, dy);
    const keep = Math.hypot(a[other][0] - a.point[0], a[other][1] - a.point[1]);
    if (length > 0) {
      moved[other] = [a.point[0] - (dx / length) * keep, a.point[1] - (dy / length) * keep];
    }
  }
  return withAnchorAt(path, i, moved);
}

/** Anchor `i` made a corner: its handles back on it (Alt+click). */
export function cornered(path: PenPath, i: number): PenPath {
  return withAnchorAt(path, i, corner(path.anchors[i].point));
}

/**
 * A new anchor at `t` of segment `i`, the curve unchanged (de Casteljau: the neighbors'
 * handles shortened, the new anchor smooth on a curve, a corner on a line).
 */
export function split(path: PenPath, i: number, t: number): PenPath {
  const [a, b] = ends(path, i);
  const ab = lerp(a.point, a.out, t);
  const bc = lerp(a.out, b.in, t);
  const cd = lerp(b.in, b.point, t);
  const abc = lerp(ab, bc, t);
  const bcd = lerp(bc, cd, t);
  const straight = isStraight(a, b);
  const added: Anchor = straight
    ? corner(lerp(a.point, b.point, t))
    : { point: lerp(abc, bcd, t), in: abc, out: bcd };
  const anchors = [...path.anchors];
  const j = (i + 1) % anchors.length;
  if (!straight) {
    anchors[i] = { ...a, out: ab };
    anchors[j] = { ...anchors[j], in: cd };
  }
  anchors.splice(i + 1, 0, added);
  return { ...path, anchors };
}

/** `path` without anchor `i`; null when too few anchors would be left to draw it (two). */
export function removed(path: PenPath, i: number): PenPath | null {
  if (path.anchors.length <= 2) return null;
  return { ...path, anchors: path.anchors.filter((_, k) => k !== i) };
}

/** Whether anchor `i` ends an open path (a click there continues it). */
export function isEnd(path: PenPath, i: number): boolean {
  return !path.closed && (i === 0 || i === path.anchors.length - 1);
}

/** `path` drawn the other way: its first anchor last. */
export function reversed(path: PenPath): PenPath {
  return {
    ...path,
    anchors: path.anchors.map((a) => ({ point: a.point, in: a.out, out: a.in })).reverse(),
  };
}

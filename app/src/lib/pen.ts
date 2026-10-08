// The Pen (P) and Direct Selection (A), ADR 0041: paths of anchors and their handles, drawn on
// the image and sent as a vector layer's path, then their anchors and handles moved again. As
// in Photoshop: a click places a corner, a drag a smooth anchor (its handles opposite each
// other), a click on the first anchor closes the path; Direct Selection drags an anchor with
// its handles, or a handle with the other one turning along unless Alt breaks them.
import * as affine from "./affine";
import type { LayerView, Matrix } from "./engine";
import type { PathSegment, PathSubpath, ShapeGeometry } from "./shapes";

export type Point = [number, number];

/** Where a path passes, and its handles on either side (null: none, a corner on that side). */
export type Anchor = { point: Point; before: Point | null; after: Point | null };

/** A run of anchors, closed back to its first or left open. */
export type PenPath = { anchors: Anchor[]; closed: boolean };

/** A path geometry, the kind a vector layer drawn with the Pen has. */
export type PathGeometry = Extract<ShapeGeometry, { kind: "path" }>;

/** How close a press must be to an anchor or handle to take it, in screen pixels. */
export const HIT_PIXELS = 6;

const same = (a: Point, b: Point) => a[0] === b[0] && a[1] === b[1];

/** The segment from anchor `a` to anchor `b`: straight unless one of them has a handle there. */
function segment(a: Anchor, b: Anchor): PathSegment {
  if (!a.after && !b.before) return { kind: "line", to: b.point };
  return { kind: "cubic", c1: a.after ?? a.point, c2: b.before ?? b.point, to: b.point };
}

/**
 * The path geometry of `paths`: each a subpath, a closed one ending with the segment back to its
 * first anchor. Runs of a single anchor draw nothing and are left out.
 */
export function geometryOf(paths: PenPath[], evenOdd = false): PathGeometry {
  const subpaths: PathSubpath[] = paths
    .filter((p) => p.anchors.length > 1)
    .map(({ anchors, closed }) => {
      const segments = anchors.slice(1).map((b, i) => segment(anchors[i], b));
      if (closed) segments.push(segment(anchors[anchors.length - 1], anchors[0]));
      return { start: anchors[0].point, segments, closed };
    });
  return { kind: "path", subpaths, evenOdd };
}

/**
 * The anchors of a path geometry: each segment's end an anchor, its control points the handles
 * of the anchors it joins (a control point on its anchor is no handle). A closed subpath whose
 * last segment comes back to its start gives that segment to its first anchor.
 */
export function pathsOf(geometry: PathGeometry): PenPath[] {
  return geometry.subpaths.map(({ start, segments, closed }) => {
    const anchors: Anchor[] = [{ point: start, before: null, after: null }];
    for (const s of segments) {
      const last = anchors[anchors.length - 1];
      if (s.kind === "cubic") {
        if (!same(s.c1, last.point)) last.after = s.c1;
        anchors.push({ point: s.to, before: same(s.c2, s.to) ? null : s.c2, after: null });
      } else {
        anchors.push({ point: s.to, before: null, after: null });
      }
    }
    if (closed && anchors.length > 1 && same(anchors[anchors.length - 1].point, start)) {
      const back = anchors.pop() as Anchor;
      anchors[0].before = back.before;
    }
    return { anchors, closed };
  });
}

/** How much of a circle's radius a cubic's handles take to draw a quarter of it. */
const KAPPA = 0.5522847498;

/**
 * The anchors of any shape's outline, for Direct Selection: a path's own, a line's two ends,
 * a polygon's or star's corners, a rectangle's corners (rounded ones each two anchors, their
 * handles drawing the quarter circles), an ellipse's four anchors. Photoshop's "turn a live
 * shape into a regular path".
 */
export function anchorsOf(geometry: ShapeGeometry): PenPath[] {
  const corner = (point: Point): Anchor => ({ point, before: null, after: null });
  switch (geometry.kind) {
    case "path":
      return pathsOf(geometry);
    case "line":
      return [{ anchors: [corner(geometry.from), corner(geometry.to)], closed: false }];
    case "polygon": {
      const n = geometry.star === null ? geometry.sides : 2 * geometry.sides;
      const anchors = Array.from({ length: n }, (_, i): Anchor => {
        const r =
          geometry.star !== null && i % 2 === 1 ? geometry.radius * geometry.star : geometry.radius;
        const a = ((geometry.rotation + (360 / n) * i) * Math.PI) / 180;
        // Up is negative y.
        return corner([geometry.center[0] + r * Math.cos(a), geometry.center[1] - r * Math.sin(a)]);
      });
      return [{ anchors, closed: true }];
    }
    case "ellipse": {
      const [cx, cy] = geometry.center;
      const [rx, ry] = geometry.radii;
      const [kx, ky] = [rx * KAPPA, ry * KAPPA];
      // Clockwise on screen from the right: right, bottom, left, top.
      const anchors: Anchor[] = [
        { point: [cx + rx, cy], before: [cx + rx, cy - ky], after: [cx + rx, cy + ky] },
        { point: [cx, cy + ry], before: [cx + kx, cy + ry], after: [cx - kx, cy + ry] },
        { point: [cx - rx, cy], before: [cx - rx, cy + ky], after: [cx - rx, cy - ky] },
        { point: [cx, cy - ry], before: [cx - kx, cy - ry], after: [cx + kx, cy - ry] },
      ];
      return [{ anchors, closed: true }];
    }
    case "rectangle": {
      const [l, t, r, b] = geometry.rect;
      const radius = Math.min(geometry.radii[0], (r - l) / 2, (b - t) / 2);
      if (radius <= 0) {
        const corners: Point[] = [
          [l, t],
          [r, t],
          [r, b],
          [l, b],
        ];
        return [{ anchors: corners.map(corner), closed: true }];
      }
      const k = radius * KAPPA;
      // Clockwise from the top edge's left end: each rounded corner an anchor on either side.
      const anchors: Anchor[] = [
        { point: [l + radius, t], before: [l + radius - k, t], after: null },
        { point: [r - radius, t], before: null, after: [r - radius + k, t] },
        { point: [r, t + radius], before: [r, t + radius - k], after: null },
        { point: [r, b - radius], before: null, after: [r, b - radius + k] },
        { point: [r - radius, b], before: [r - radius + k, b], after: null },
        { point: [l + radius, b], before: null, after: [l + radius - k, b] },
        { point: [l, b - radius], before: [l, b - radius + k], after: null },
        { point: [l, t + radius], before: null, after: [l, t + radius - k] },
      ];
      return [{ anchors, closed: true }];
    }
  }
}

/** The SVG path of `paths`, mapped by `map` (affine, as the view is: curves map exactly). */
export function svgOf(paths: PenPath[], map: (x: number, y: number) => Point): string {
  const p = ([x, y]: Point) => {
    const [vx, vy] = map(x, y);
    return `${vx.toFixed(2)} ${vy.toFixed(2)}`;
  };
  return geometryOf(paths)
    .subpaths.map(({ start, segments, closed }) => {
      const parts = segments.map((s) =>
        s.kind === "line" ? `L${p(s.to)}` : `C${p(s.c1)} ${p(s.c2)} ${p(s.to)}`,
      );
      return `M${p(start)} ${parts.join(" ")}${closed ? " Z" : ""}`;
    })
    .join(" ");
}

// --- Drawing with the Pen ------------------------------------------------------------------

/**
 * A press of the Pen at `at` on `path` being drawn (null: none yet): the path with a new
 * corner there, or closed when `at` is within `reach` of its first anchor (two anchors at
 * least). The anchor the drag that may follow shapes is its last, or its first once closed.
 */
export function press(path: PenPath | null, at: Point, reach: number): PenPath {
  if (!path) return { anchors: [{ point: at, before: null, after: null }], closed: false };
  const first = path.anchors[0];
  if (
    path.anchors.length > 1 &&
    Math.hypot(at[0] - first.point[0], at[1] - first.point[1]) <= reach
  ) {
    return { ...path, closed: true };
  }
  return { ...path, anchors: [...path.anchors, { point: at, before: null, after: null }] };
}

/**
 * The drag after a press, to `to`: the anchor just placed (or the first, once the path is
 * closed) gets its handle after it there and the one before it opposite (a smooth anchor); Alt
 * (`broken`) leaves the handle before it as it was.
 */
export function pull(path: PenPath, to: Point, broken: boolean): PenPath {
  const index = path.closed ? 0 : path.anchors.length - 1;
  const anchors = path.anchors.map((a, i): Anchor => {
    if (i !== index) return a;
    const opposite: Point = [2 * a.point[0] - to[0], 2 * a.point[1] - to[1]];
    return { ...a, after: to, before: broken ? a.before : opposite };
  });
  return { ...path, anchors };
}

/** The path without its last anchor (Backspace while drawing); null when none would be left. */
export function withoutLast(path: PenPath): PenPath | null {
  if (path.anchors.length <= 1) return null;
  return { ...path, anchors: path.anchors.slice(0, -1) };
}

/** Whether `path` draws something: two anchors at least, not all in one place. */
export function drawable(path: PenPath): boolean {
  if (path.anchors.length < 2) return false;
  const first = path.anchors[0].point;
  return path.anchors.some((a) => !same(a.point, first) || a.before !== null || a.after !== null);
}

// --- Direct Selection ------------------------------------------------------------------------

/** What a press of Direct Selection takes: an anchor, or one of its handles. */
export type Hit = { path: number; anchor: number; part: "point" | "before" | "after" };

/** The anchor or handle at `at` within `reach`, handles first (they are drawn over anchors). */
export function hitAt(paths: PenPath[], at: Point, reach: number): Hit | null {
  const near = (p: Point | null) => p !== null && Math.hypot(p[0] - at[0], p[1] - at[1]) <= reach;
  for (const part of ["after", "before", "point"] as const) {
    for (let path = 0; path < paths.length; path++) {
      const anchors = paths[path].anchors;
      for (let anchor = 0; anchor < anchors.length; anchor++) {
        if (near(anchors[anchor][part])) return { path, anchor, part };
      }
    }
  }
  return null;
}

/**
 * `paths` with what `hit` took moved to `to`: an anchor and its handles together; a handle,
 * the anchor's other handle turning to stay opposite it at its own length, unless `broken`
 * (Alt), or unless the anchor has no other handle.
 */
export function moved(paths: PenPath[], hit: Hit, to: Point, broken: boolean): PenPath[] {
  return paths.map((path, p) => {
    if (p !== hit.path) return path;
    const anchors = path.anchors.map((a, i): Anchor => {
      if (i !== hit.anchor) return a;
      if (hit.part === "point") {
        const [dx, dy] = [to[0] - a.point[0], to[1] - a.point[1]];
        const by = (h: Point | null): Point | null => (h ? [h[0] + dx, h[1] + dy] : null);
        return { point: to, before: by(a.before), after: by(a.after) };
      }
      const other = hit.part === "after" ? "before" : "after";
      const otherHandle = a[other];
      let turned = otherHandle;
      if (!broken && otherHandle) {
        const length = Math.hypot(otherHandle[0] - a.point[0], otherHandle[1] - a.point[1]);
        const [dx, dy] = [to[0] - a.point[0], to[1] - a.point[1]];
        const d = Math.hypot(dx, dy);
        if (d > 0) turned = [a.point[0] - (dx / d) * length, a.point[1] - (dy / d) * length];
      }
      return { ...a, [hit.part]: to, [other]: turned } as Anchor;
    });
    return { ...path, anchors };
  });
}

// --- Where a layer's content is --------------------------------------------------------------

/**
 * The map from layer `id`'s content to the document: its transform, then each group's around
 * it. Null when the layer is not found or one of them is in perspective (ADR 0038): Direct
 * Selection edits affine placements only.
 */
export function contentToDocument(layers: LayerView[], id: number): Matrix | null {
  const chain = (list: LayerView[]): LayerView[] | null => {
    for (const layer of list) {
      if (layer.id === id) return [layer];
      const inside = chain(layer.children);
      if (inside) return [...inside, layer];
    }
    return null;
  };
  const found = chain(layers);
  if (!found || found.some((l) => l.perspective)) return null;
  return found.reduce<Matrix>((m, layer) => affine.andThen(m, layer.transform), affine.IDENTITY);
}

// Edit > Perspective Warp (ADR 0044), as in Photoshop. In Layout mode, planes are drawn over the
// image (a drag draws a rectangle) and their corners dragged onto its lines; a corner dropped by
// another plane's corner joins it, so that planes share their edges. In Warp mode, the corners are
// moved and the image follows, each plane by its own homography. Planes are their corners where
// they were laid out and where they were moved (content pixels), each plane four indices into
// them, in order around it: slopshop_core::warp::Planes, which validates and draws what is sent.

import type { EditRequest } from "./engine";
import * as homography from "./homography";

export type Point = [number, number];
export type Quad = [number, number, number, number];

/** Perspective Warp's planes (see above). */
export type Planes = { layout: Point[]; warped: Point[]; planes: Quad[] };

/** Layout draws and fits the planes over the unwarped image; Warp moves their corners. */
export type PerspectiveMode = "layout" | "warp";

/** Most planes (slopshop_core::warp::MAX_PLANES). */
export const MAX_PLANES = 32;

/** The smallest side of a plane drawn, in content pixels. */
export const MIN_SIDE = 4;

/** No plane drawn yet. */
export const NO_PLANES: Planes = { layout: [], warped: [], planes: [] };

/** The corners each mode shows and moves. */
function pointsOf(planes: Planes, mode: PerspectiveMode): Point[] {
  return mode === "layout" ? planes.layout : planes.warped;
}

/** Plane `k`'s corners in `mode`. */
export function quadOf(planes: Planes, k: number, mode: PerspectiveMode): Point[] {
  const points = pointsOf(planes, mode);
  return planes.planes[k].map((i) => points[i]);
}

/** The quad's area (either way round). */
function area(quad: Point[]): number {
  let twice = 0;
  for (let k = 0; k < 4; k++) {
    const [ax, ay] = quad[k];
    const [bx, by] = quad[(k + 1) % 4];
    twice += ax * by - bx * ay;
  }
  return Math.abs(twice) / 2;
}

/** Whether a quad can be a plane: convex, at least a pixel (as the engine requires). */
export function isPlane(quad: Point[]): boolean {
  return homography.isConvex(quad) && area(quad) >= 1;
}

/** Whether every plane is one, as laid out and as moved. */
export function isValid(planes: Planes): boolean {
  return planes.planes.every(
    (_, k) => isPlane(quadOf(planes, k, "layout")) && isPlane(quadOf(planes, k, "warp")),
  );
}

/** The corners no plane uses dropped, the planes' indices renumbered. */
function compact(planes: Planes): Planes {
  const used = new Set(planes.planes.flat());
  const index = new Map<number, number>();
  const layout: Point[] = [];
  const warped: Point[] = [];
  planes.layout.forEach((p, i) => {
    if (!used.has(i)) return;
    index.set(i, layout.length);
    layout.push(p);
    warped.push(planes.warped[i]);
  });
  const quads = planes.planes.map((q) => q.map((i) => index.get(i) ?? 0) as Quad);
  return { layout, warped, planes: quads };
}

/** The squared distance between two points. */
function distance2([ax, ay]: Point, [bx, by]: Point): number {
  return (ax - bx) ** 2 + (ay - by) ** 2;
}

/**
 * A plane drawn as the rectangle from `a` to `b` (content pixels), its corners joined to other
 * planes' corners within `snap` of them (as laid out), so that it shares their edges: as in
 * Photoshop, a plane drawn next to another snaps to it. Null when it is too small, there are
 * MAX_PLANES already, or it cannot be a plane.
 */
export function addPlane(planes: Planes, a: Point, b: Point, snap: number): Planes | null {
  if (planes.planes.length >= MAX_PLANES) return null;
  const [x0, x1] = [Math.min(a[0], b[0]), Math.max(a[0], b[0])];
  const [y0, y1] = [Math.min(a[1], b[1]), Math.max(a[1], b[1])];
  if (x1 - x0 < MIN_SIDE || y1 - y0 < MIN_SIDE) return null;
  const rect: Point[] = [
    [x0, y0],
    [x1, y0],
    [x1, y1],
    [x0, y1],
  ];
  const withCorners = (joined: (number | null)[]): Planes => {
    const layout = [...planes.layout];
    const warped = [...planes.warped];
    const quad = rect.map((p, k) => {
      const j = joined[k];
      if (j !== null) return j;
      layout.push(p);
      warped.push(p);
      return layout.length - 1;
    }) as Quad;
    return { layout, warped, planes: [...planes.planes, quad] };
  };
  // Each corner joined to the nearest corner within reach, each corner at most once.
  const taken = new Set<number>();
  const joined = rect.map((p) => {
    let best: number | null = null;
    let bestDistance = snap * snap;
    planes.layout.forEach((q, i) => {
      const d = distance2(p, q);
      if (!taken.has(i) && d <= bestDistance) {
        best = i;
        bestDistance = d;
      }
    });
    if (best !== null) taken.add(best);
    return best;
  });
  for (const attempt of [joined, rect.map(() => null)]) {
    const next = withCorners(attempt);
    if (isValid(next)) return next;
  }
  return null;
}

/**
 * Corner `index` dragged to `to` (content pixels). In Layout mode it is laid out there, and where
 * it was moved follows by as much; in Warp mode, it is moved there. Null when a plane would stop
 * being one (folded, or not convex): the drag stays where it was.
 */
export function moveCorner(
  planes: Planes,
  index: number,
  to: Point,
  mode: PerspectiveMode,
): Planes | null {
  const layout = [...planes.layout];
  const warped = [...planes.warped];
  if (mode === "layout") {
    const [dx, dy] = [to[0] - layout[index][0], to[1] - layout[index][1]];
    layout[index] = to;
    warped[index] = [warped[index][0] + dx, warped[index][1] + dy];
  } else {
    warped[index] = to;
  }
  const next = { ...planes, layout, warped };
  return isValid(next) ? next : null;
}

/**
 * After a corner's drag in Layout mode: joined to the nearest corner within `snap` of it (as laid
 * out) of a plane it is not in, as in Photoshop, so that the planes share it. Unchanged when there
 * is none, or when joining would make a plane invalid.
 */
export function joinCorner(planes: Planes, index: number, snap: number): Planes {
  const own = planes.planes.filter((q) => q.includes(index));
  let best: number | null = null;
  let bestDistance = snap * snap;
  planes.layout.forEach((p, i) => {
    if (i === index || own.some((q) => q.includes(i))) return;
    const d = distance2(p, planes.layout[index]);
    if (d <= bestDistance) {
      best = i;
      bestDistance = d;
    }
  });
  if (best === null) return planes;
  const target: number = best;
  const quads = planes.planes.map((q) => q.map((i) => (i === index ? target : i)) as Quad);
  const next = compact({ ...planes, planes: quads });
  return isValid(next) ? next : planes;
}

/** Plane `k` taken away, with its corners no other plane uses. */
export function removePlane(planes: Planes, k: number): Planes {
  return compact({ ...planes, planes: planes.planes.filter((_, i) => i !== k) });
}

/** The last plane (drawn on top) holding content point `p` in `mode`, or null. */
export function planeAt(planes: Planes, mode: PerspectiveMode, [x, y]: Point): number | null {
  for (let k = planes.planes.length - 1; k >= 0; k--) {
    const quad = quadOf(planes, k, mode);
    const sides = quad.map(([ax, ay], i) => {
      const [bx, by] = quad[(i + 1) % 4];
      return (bx - ax) * (y - ay) - (by - ay) * (x - ax);
    });
    if (sides.every((s) => s >= 0) || sides.every((s) => s <= 0)) return k;
  }
  return null;
}

/** The unit square, the planes' parameters. */
const UNIT = { left: 0, top: 0, right: 1, bottom: 1 };

/**
 * Plane `k`'s lines in `mode` (content pixels): its four edges, then `divisions - 1` lines across
 * each way, each from one edge to the other, as the plane's homography places them in Warp mode.
 */
export function planeLines(
  planes: Planes,
  k: number,
  mode: PerspectiveMode,
  divisions = 3,
): [Point, Point][] {
  const laid = quadOf(planes, k, "layout");
  const quad = quadOf(planes, k, mode);
  // Lines across the plane as laid out (straight: a bilinear map's lines are), then mapped.
  const bilinear = (u: number, v: number): Point => {
    const [a, b, c, d] = laid;
    const w = [(1 - u) * (1 - v), u * (1 - v), u * v, (1 - u) * v];
    return [
      w[0] * a[0] + w[1] * b[0] + w[2] * c[0] + w[3] * d[0],
      w[0] * a[1] + w[1] * b[1] + w[2] * c[1] + w[3] * d[1],
    ];
  };
  const toLayout = homography.rectToQuad(UNIT, laid);
  const toQuad = homography.rectToQuad(UNIT, quad);
  const fromLayout = toLayout && homography.invert(toLayout);
  const map =
    mode === "warp" && fromLayout && toQuad ? homography.andThen(fromLayout, toQuad) : null;
  const place = (p: Point): Point => (map ? homography.apply(map, p[0], p[1]) : p);
  const lines: [Point, Point][] = quad.map((p, i) => [p, quad[(i + 1) % 4]]);
  for (let i = 1; i < divisions; i++) {
    const s = i / divisions;
    lines.push([place(bilinear(s, 0)), place(bilinear(s, 1))]);
    lines.push([place(bilinear(0, s)), place(bilinear(1, s))]);
  }
  return lines;
}

/** Whether two sets of planes are the same. */
export function samePlanes(a: Planes, b: Planes): boolean {
  const same = (p: Point[], q: Point[]) =>
    p.length === q.length && p.every(([x, y], i) => x === q[i][0] && y === q[i][1]);
  return (
    same(a.layout, b.layout) &&
    same(a.warped, b.warped) &&
    a.planes.length === b.planes.length &&
    a.planes.every((q, i) => q.every((c, j) => c === b.planes[i][j]))
  );
}

/**
 * The request warping layer `id` by `planes` (none: its warp taken away, as Layout mode shows the
 * layer unwarped and as no plane warps nothing).
 */
export function planesRequest(id: number, planes: Planes | null): EditRequest {
  if (!planes || planes.planes.length === 0) return { kind: "perspectiveWarpLayer", id };
  return {
    kind: "perspectiveWarpLayer",
    id,
    planes: { layout: planes.layout, warped: planes.warped, planes: planes.planes },
  };
}

// Projective maps of the plane (ADR 0038), as the engine's `Projective`: nine numbers
// [a, b, c, d, e, f, g, h, i], a point (x, y) going to ((a·x + c·y + e) / w, (b·x + d·y + f) / w)
// with w = g·x + h·y + i. Free Transform's Distort and Perspective place the box's corners
// freely: the map is the one sending the box to that quad.
//
// No imports but types: this module also runs under Node for its tests.

import type { Matrix } from "./engine";

export type Homography = [number, number, number, number, number, number, number, number, number];

type Point = [number, number];

/** A rectangle `{ left, top, right, bottom }`. */
type Rect = { left: number; top: number; right: number; bottom: number };

/** The map of an affine matrix `[a, b, c, d, e, f]`. */
export function fromAffine([a, b, c, d, e, f]: Matrix): Homography {
  return [a, b, c, d, e, f, 0, 0, 1];
}

/** The affine matrix of `h` when it is one (its last row 0 0 1), else null. */
export function toAffine(h: Homography): Matrix | null {
  const [a, b, c, d, e, f, g, hh, i] = h;
  if (g !== 0 || hh !== 0 || i === 0) return null;
  return [a / i, b / i, c / i, d / i, e / i, f / i];
}

/** The image of (`x`, `y`); not finite on the horizon line. */
export function apply(m: Homography, x: number, y: number): Point {
  const [a, b, c, d, e, f, g, h, i] = m;
  const w = g * x + h * y + i;
  return [(a * x + c * y + e) / w, (b * x + d * y + f) / w];
}

/** `m` then `n`: the map p ↦ n(m(p)), normalized so that its last number is 1 when it can be. */
export function andThen(m: Homography, n: Homography): Homography {
  const [a1, b1, c1, d1, e1, f1, g1, h1, i1] = m;
  const [a2, b2, c2, d2, e2, f2, g2, h2, i2] = n;
  const row = (r: number[], col: number[]) => r[0] * col[0] + r[1] * col[1] + r[2] * col[2];
  const [rx, ry, rw] = [
    [a2, c2, e2],
    [b2, d2, f2],
    [g2, h2, i2],
  ];
  const [colX, colY, colW] = [
    [a1, b1, g1],
    [c1, d1, h1],
    [e1, f1, i1],
  ];
  const out: Homography = [
    row(rx, colX),
    row(ry, colX),
    row(rx, colY),
    row(ry, colY),
    row(rx, colW),
    row(ry, colW),
    row(rw, colX),
    row(rw, colY),
    row(rw, colW),
  ];
  return normalized(out);
}

function normalized(m: Homography): Homography {
  const i = m[8];
  return i !== 0 && Number.isFinite(i) ? (m.map((v) => v / i) as Homography) : m;
}

/** The four corners of `rect`: top left, top right, bottom right, bottom left. */
export function corners(rect: Rect): [Point, Point, Point, Point] {
  return [
    [rect.left, rect.top],
    [rect.right, rect.top],
    [rect.right, rect.bottom],
    [rect.left, rect.bottom],
  ];
}

/**
 * The map sending the corners of `rect` (top left, top right, bottom right, bottom left) to
 * `quad`, in that order (Heckbert's square to quad); null when three points of `quad` are on one
 * line or `rect` is empty.
 */
export function rectToQuad(rect: Rect, quad: Point[]): Homography | null {
  const w = rect.right - rect.left;
  const h = rect.bottom - rect.top;
  if (!(w > 0 && h > 0) || quad.length !== 4) return null;
  const [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = quad;
  const sx = x0 - x1 + x2 - x3;
  const sy = y0 - y1 + y2 - y3;
  const [dx1, dy1] = [x1 - x2, y1 - y2];
  const [dx2, dy2] = [x3 - x2, y3 - y2];
  const den = dx1 * dy2 - dx2 * dy1;
  if (den === 0 || !Number.isFinite(den)) return null;
  const g = (sx * dy2 - dx2 * sy) / den;
  const hh = (dx1 * sy - sx * dy1) / den;
  const square: Homography = [
    x1 - x0 + g * x1,
    y1 - y0 + g * y1,
    x3 - x0 + hh * x3,
    y3 - y0 + hh * y3,
    x0,
    y0,
    g,
    hh,
    1,
  ];
  const toSquare: Homography = [1 / w, 0, 0, 1 / h, -rect.left / w, -rect.top / h, 0, 0, 1];
  const m = andThen(toSquare, square);
  // Three points on a line flatten the plane: no inverse.
  return m.every(Number.isFinite) && invert(m) !== null ? m : null;
}

/**
 * Whether `quad` (four corners in order) is a convex quadrilateral turning one way: a place a
 * rectangle can be put in perspective to, every point of it on the near side of the horizon.
 */
export function isConvex(quad: Point[]): boolean {
  if (quad.length !== 4) return false;
  let sign = 0;
  for (let k = 0; k < 4; k++) {
    const [ax, ay] = quad[k];
    const [bx, by] = quad[(k + 1) % 4];
    const [cx, cy] = quad[(k + 2) % 4];
    const cross = (bx - ax) * (cy - by) - (by - ay) * (cx - bx);
    if (Math.abs(cross) < 1e-9) return false;
    const s = Math.sign(cross);
    if (sign !== 0 && s !== sign) return false;
    sign = s;
  }
  return true;
}

/** The inverse map, or null when it has none. */
export function invert(m: Homography): Homography | null {
  const [a, b, c, d, e, f, g, h, i] = m;
  const det = a * (d * i - f * h) - c * (b * i - f * g) + e * (b * h - d * g);
  if (!det || !Number.isFinite(det)) return null;
  return normalized([
    (d * i - f * h) / det,
    (f * g - b * i) / det,
    (e * h - c * i) / det,
    (a * i - e * g) / det,
    (c * f - e * d) / det,
    (e * b - a * f) / det,
    (b * h - d * g) / det,
    (c * g - a * h) / det,
    (a * d - c * b) / det,
  ]);
}

/**
 * Perspective's corner drag, as in Photoshop: corner `k` of `quad` moved by (`dx`, `dy`) along the
 * side it shares with the corner it is paired with on the axis it moves most, which moves the
 * other way: the side stays centered (a symmetric trapezoid from a rectangle).
 */
export function perspectiveDrag(quad: Point[], k: number, dx: number, dy: number): Point[] {
  // Top and bottom sides pair 0–1 and 3–2; left and right sides pair 0–3 and 1–2.
  const along = Math.abs(dx) >= Math.abs(dy) ? "x" : "y";
  const partner = along === "x" ? [1, 0, 3, 2][k] : [3, 2, 1, 0][k];
  const out = quad.map(([x, y]) => [x, y] as Point);
  if (along === "x") {
    out[k][0] += dx;
    out[partner][0] -= dx;
  } else {
    out[k][1] += dy;
    out[partner][1] -= dy;
  }
  return out;
}

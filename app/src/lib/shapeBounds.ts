// A vector layer's W, H, X and Y in the Properties panel (ADR 0041), as Photoshop's: the
// bounds of the shape's outline (its stroke left out) in document pixels, and the geometry a
// typed value gives. Only where the layer maps its content without rotation, skew or
// perspective: the bounds are then a box of the layer's space mapped to the document's.
import * as affine from "./affine";
import type { Matrix } from "./engine";
import { outlinePoints, type Box, type ShapeGeometry } from "./shapes";

type Point = [number, number];

/** The values a cubic Bézier's coordinate (`p0` to `p3`) takes where it turns back. */
function cubicExtremes(p0: number, p1: number, p2: number, p3: number): number[] {
  // The derivative, over 3: a·t² + b·t + c.
  const [d0, d1, d2] = [p1 - p0, p2 - p1, p3 - p2];
  const a = d0 - 2 * d1 + d2;
  const b = 2 * (d1 - d0);
  const c = d0;
  const roots: number[] = [];
  if (Math.abs(a) < 1e-12) {
    if (Math.abs(b) > 1e-12) roots.push(-c / b);
  } else {
    const disc = b * b - 4 * a * c;
    if (disc >= 0) {
      const sq = Math.sqrt(disc);
      roots.push((-b + sq) / (2 * a), (-b - sq) / (2 * a));
    }
  }
  return roots
    .filter((t) => t > 0 && t < 1)
    .map((t) => {
      const u = 1 - t;
      return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3;
    });
}

function boxOf(points: Point[]): Box {
  return {
    left: Math.min(...points.map((p) => p[0])),
    top: Math.min(...points.map((p) => p[1])),
    right: Math.max(...points.map((p) => p[0])),
    bottom: Math.max(...points.map((p) => p[1])),
  };
}

/** The bounds of `geometry`'s outline, in its layer's space (a path's curves, not handles). */
export function geometryBounds(geometry: ShapeGeometry): Box {
  switch (geometry.kind) {
    case "rectangle": {
      const [left, top, right, bottom] = geometry.rect;
      return { left, top, right, bottom };
    }
    case "ellipse": {
      const [cx, cy] = geometry.center;
      const [rx, ry] = geometry.radii;
      return { left: cx - rx, top: cy - ry, right: cx + rx, bottom: cy + ry };
    }
    case "polygon":
    case "line":
      return boxOf(outlinePoints(geometry));
    case "path": {
      const xs: number[] = [];
      const ys: number[] = [];
      for (const subpath of geometry.subpaths) {
        let from = subpath.start;
        xs.push(from[0]);
        ys.push(from[1]);
        for (const s of subpath.segments) {
          const to: Point = s.length === 2 ? [s[0], s[1]] : [s[4], s[5]];
          if (s.length === 6) {
            xs.push(...cubicExtremes(from[0], s[0], s[2], to[0]));
            ys.push(...cubicExtremes(from[1], s[1], s[3], to[1]));
          }
          xs.push(to[0]);
          ys.push(to[1]);
          from = to;
        }
      }
      if (xs.length === 0) return { left: 0, top: 0, right: 0, bottom: 0 };
      return {
        left: Math.min(...xs),
        top: Math.min(...ys),
        right: Math.max(...xs),
        bottom: Math.max(...ys),
      };
    }
  }
}

/** Whether `matrix` keeps boxes boxes: no rotation nor skew. */
function axisAligned(m: Matrix): boolean {
  return Math.abs(m[1]) < 1e-9 && Math.abs(m[2]) < 1e-9 && m[0] !== 0 && m[3] !== 0;
}

function mapBox(m: Matrix, box: Box): Box {
  return boxOf([affine.apply(m, box.left, box.top), affine.apply(m, box.right, box.bottom)]);
}

/** X, Y, W and H as the Properties panel shows them. */
export type Bounds = { x: number; y: number; width: number; height: number };
export type BoundsField = keyof Bounds;

/**
 * `geometry`'s bounds in the document, `matrix` mapping its layer there; null when the matrix
 * rotates or skews (or the layer is in perspective: no matrix).
 */
export function documentBounds(geometry: ShapeGeometry, matrix: Matrix | null): Bounds | null {
  if (!matrix || !axisAligned(matrix)) return null;
  const box = mapBox(matrix, geometryBounds(geometry));
  return { x: box.left, y: box.top, width: box.right - box.left, height: box.bottom - box.top };
}

/**
 * `geometry` with its document bounds' `field` set to `value`, as Photoshop's fields: X and Y
 * move the shape, W and H scale it from its top left corner. A rectangle keeps its corner
 * radii; a polygon stays regular, its other side following. Null when nothing can change (a
 * size of zero or less, a side of zero to scale, a matrix that rotates or skews).
 */
export function withBounds(
  geometry: ShapeGeometry,
  matrix: Matrix | null,
  field: BoundsField,
  value: number,
): ShapeGeometry | null {
  const bounds = documentBounds(geometry, matrix);
  const inverse = matrix && affine.invert(matrix);
  if (!bounds || !inverse || !Number.isFinite(value)) return null;
  const next = { ...bounds, [field]: value };
  if (field === "width" || field === "height") {
    if (value <= 0 || bounds[field] <= 0) return null;
    if (geometry.kind === "polygon") {
      const other = field === "width" ? "height" : "width";
      next[other] = bounds[other] * (value / bounds[field]);
    }
  }
  const from = geometryBounds(geometry);
  const to = mapBox(inverse, {
    left: next.x,
    top: next.y,
    right: next.x + next.width,
    bottom: next.y + next.height,
  });
  const sx = from.right > from.left ? (to.right - to.left) / (from.right - from.left) : 1;
  const sy = from.bottom > from.top ? (to.bottom - to.top) / (from.bottom - from.top) : 1;
  const map = ([x, y]: Point): Point => [
    to.left + (x - from.left) * sx,
    to.top + (y - from.top) * sy,
  ];
  switch (geometry.kind) {
    case "rectangle": {
      const [l, t] = map([geometry.rect[0], geometry.rect[1]]);
      const [r, b] = map([geometry.rect[2], geometry.rect[3]]);
      return { ...geometry, rect: [l, t, r, b] };
    }
    case "ellipse":
      return {
        ...geometry,
        center: map(geometry.center),
        radii: [geometry.radii[0] * sx, geometry.radii[1] * sy],
      };
    case "polygon":
      return { ...geometry, center: map(geometry.center), radius: geometry.radius * sx };
    case "line":
      return { ...geometry, from: map(geometry.from), to: map(geometry.to) };
    case "path":
      return {
        ...geometry,
        subpaths: geometry.subpaths.map((s) => ({
          ...s,
          start: map(s.start),
          segments: s.segments.map((seg) => {
            const out: number[] = [];
            for (let i = 0; i < seg.length; i += 2) out.push(...map([seg[i], seg[i + 1]]));
            return out;
          }),
        })),
      };
  }
}

// 2D affine maps for on-canvas interaction (Free Transform's handles): the same convention as
// `Affine` in slopshop-core, [a, b, c, d, e, f] with (x, y) ↦ (a·x + c·y + e, b·x + d·y + f).
// Only gesture geometry lives here; layers' transforms are composed by the engine.

import type { Matrix } from "./engine";

export const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];

/** `m`, then `n`. Not named `then`: a module exporting `then` passes for a promise. */
export function andThen(m: Matrix, n: Matrix): Matrix {
  const [a, b, c, d, e, f] = m;
  const [na, nb, nc, nd, ne, nf] = n;
  return [
    na * a + nc * b,
    nb * a + nd * b,
    na * c + nc * d,
    nb * c + nd * d,
    na * e + nc * f + ne,
    nb * e + nd * f + nf,
  ];
}

export function apply(m: Matrix, x: number, y: number): [number, number] {
  return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
}

/** The inverse map, or null when `m` is not invertible. */
export function invert(m: Matrix): Matrix | null {
  const [a, b, c, d, e, f] = m;
  const det = a * d - b * c;
  if (!Number.isFinite(det) || det === 0) return null;
  const [ia, ib, ic, id] = [d / det, -b / det, -c / det, a / det];
  return [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)];
}

export const translation = (x: number, y: number): Matrix => [1, 0, 0, 1, x, y];
export const scaling = (sx: number, sy: number): Matrix => [sx, 0, 0, sy, 0, 0];

/** A rotation by `radians`, clockwise on screen (y points down). */
export function rotation(radians: number): Matrix {
  const [sin, cos] = [Math.sin(radians), Math.cos(radians)];
  return [cos, sin, -sin, cos, 0, 0];
}

/** `m` applied about the point (`x`, `y`) instead of the origin. */
export function about(m: Matrix, x: number, y: number): Matrix {
  return andThen(andThen(translation(-x, -y), m), translation(x, y));
}

export function isIdentity(m: Matrix): boolean {
  return m.every((v, i) => v === IDENTITY[i]);
}

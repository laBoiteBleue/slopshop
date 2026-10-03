// Free Transform's numbers, as Photoshop's options bar shows them: where the reference point
// is, the width and height as shares of the original, the angle and the skew. A transform (an
// affine map [a, b, c, d, e, f], (x, y) ↦ (a·x + c·y + e, b·x + d·y + f), as in affine.ts)
// is taken apart into them and put back together from them.
//
// No imports: this module also runs under Node for its tests (`npm test`).

export type Map6 = [number, number, number, number, number, number];

export type TransformValues = {
  /** Where the reference point is, document pixels. */
  x: number;
  y: number;
  /** Width and height as shares of the original (1: 100 %); negative when flipped. */
  width: number;
  height: number;
  /** Rotation, radians, clockwise on screen. */
  angle: number;
  /** Horizontal skew, radians: how far the sides lean from upright. */
  skew: number;
};

/**
 * The values of `m` with its reference point at `pivot` (in the original's coordinates):
 * `m`'s linear part is a rotation of a skew of a scale, `rotation · [[1, tan(skew)], [0, 1]]
 * · scale(width, height)`.
 */
export function decompose(m: Map6, pivot: [number, number]): TransformValues {
  const [a, b, c, d, e, f] = m;
  const width = Math.hypot(a, b);
  const angle = Math.atan2(b, a);
  const [cos, sin] = [Math.cos(angle), Math.sin(angle)];
  const height = width === 0 ? 0 : (a * d - b * c) / width;
  const shear = c * cos + d * sin;
  const skew = height === 0 ? 0 : Math.atan(shear / height);
  return {
    x: a * pivot[0] + c * pivot[1] + e,
    y: b * pivot[0] + d * pivot[1] + f,
    width,
    height,
    angle,
    skew,
  };
}

/** The transform whose values are `v`, its reference point at `pivot` (see `decompose`). */
export function compose(v: TransformValues, pivot: [number, number]): Map6 {
  const [cos, sin] = [Math.cos(v.angle), Math.sin(v.angle)];
  const k = Math.tan(v.skew);
  // rotation · skew · scale.
  const [la, lb] = [cos * v.width, sin * v.width];
  const [lc, ld] = [(cos * k - sin) * v.height, (sin * k + cos) * v.height];
  return [
    la,
    lb,
    lc,
    ld,
    v.x - (la * pivot[0] + lc * pivot[1]),
    v.y - (lb * pivot[0] + ld * pivot[1]),
  ];
}

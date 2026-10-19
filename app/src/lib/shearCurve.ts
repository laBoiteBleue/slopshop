// Filter > Distort > Shear's curve (the engine's `ShearCurve`): how far each row of the frame
// moves sideways, a smooth curve through 2 to 16 points `[row, offset]`, `row` in percent of the
// frame's height from its top (the first point at 0, the last at 100), `offset` in percent of
// half its width (-100 to 100, to the right). Its editor's geometry: a grid, the line through
// the points, points added, dragged and dragged off it.

/** A point of the curve: its row and its offset, in percent. */
export type ShearPoint = [row: number, offset: number];

/** Points at most (the engine's `SHEAR_POINTS`). */
export const SHEAR_POINTS = 16;

/** Photoshop's Default: the straight vertical line, no row moved. */
export const STRAIGHT: ShearPoint[] = [
  [0, 0],
  [100, 0],
];

/** How close two points' rows may be, in percent of the height. */
export const MIN_GAP = 1;

/** How far outside the grid a point dragged off it is removed, in pixels. */
export const OFF_GRID = 12;

/** How close to a point a press grabs it, in pixels. */
export const GRAB = 6;

/** The client rectangle of the grid. */
export type Box = { left: number; top: number; width: number; height: number };

/** The points in `values` (a row, an offset, …). */
export function shearPoints(values: number[]): ShearPoint[] {
  const points: ShearPoint[] = [];
  for (let i = 0; i + 1 < values.length; i += 2) points.push([values[i], values[i + 1]]);
  return points;
}

/** `points` as a filter's values. */
export function shearValues(points: ShearPoint[]): number[] {
  return points.flat();
}

/** Whether `points` make a curve the engine takes (`ShearCurve::new`). */
export function validShearPoints(points: ShearPoint[]): boolean {
  const n = points.length;
  return (
    n >= 2 &&
    n <= SHEAR_POINTS &&
    points[0][0] === 0 &&
    points[n - 1][0] === 100 &&
    points.every(
      ([row, offset]) => Number.isFinite(row) && Number.isFinite(offset) && Math.abs(offset) <= 100,
    ) &&
    points.every((p, i) => i === 0 || points[i - 1][0] < p[0])
  );
}

/**
 * The curve's offset (percent) at row `row` (percent, clamped to 0–100): the natural cubic
 * spline through the points, as the engine's, within -100 to 100.
 */
export function shearOffset(points: ShearPoint[], row: number): number {
  return curveOf(points)(row);
}

/** The curve as a function of the row, its spline solved once (for drawing it). */
export function curveOf(points: ShearPoint[]): (row: number) => number {
  const n = points.length;
  const x = points.map((p) => p[0] / 100);
  const y = points.map((p) => p[1] / 100);
  // The tridiagonal system of a natural spline (zero second derivative at both ends).
  const second = new Array<number>(n).fill(0);
  if (n > 2) {
    const h = x.slice(1).map((v, i) => v - x[i]);
    const slope = h.map((d, i) => (y[i + 1] - y[i]) / d);
    const diagonal = new Array<number>(n).fill(0);
    const rhs = new Array<number>(n).fill(0);
    for (let i = 1; i < n - 1; i++) {
      diagonal[i] = 2 * (h[i - 1] + h[i]);
      rhs[i] = 6 * (slope[i] - slope[i - 1]);
    }
    for (let i = 2; i < n - 1; i++) {
      const m = h[i - 1] / diagonal[i - 1];
      diagonal[i] -= m * h[i - 1];
      rhs[i] -= m * rhs[i - 1];
    }
    for (let i = n - 2; i >= 1; i--) second[i] = (rhs[i] - h[i] * second[i + 1]) / diagonal[i];
  }
  return (row: number) => {
    const t = Math.min(Math.max(Number.isNaN(row) ? 0 : row / 100, 0), 1);
    let k = 1;
    while (k < n - 1 && x[k] <= t) k++;
    const span = x[k] - x[k - 1];
    const a = (x[k] - t) / span;
    const b = (t - x[k - 1]) / span;
    const v =
      a * y[k - 1] +
      b * y[k] +
      (((a * a * a - a) * second[k - 1] + (b * b * b - b) * second[k]) * span * span) / 6;
    return Math.min(Math.max(v, -1), 1) * 100;
  };
}

/** The point of the grid at client point (`x`, `y`): within it, to a tenth of a percent. */
export function gridPoint(box: Box, x: number, y: number): ShearPoint {
  const along = (v: number, start: number, size: number) =>
    size > 0 ? Math.min(Math.max((v - start) / size, 0), 1) : 0.5;
  const tenth = (v: number) => Math.round(v * 10) / 10;
  return [
    tenth(along(y, box.top, box.height) * 100),
    tenth(along(x, box.left, box.width) * 200 - 100),
  ];
}

/** Where point `p` is shown in the grid at `box`, in client pixels. */
export function clientPoint(box: Box, [row, offset]: ShearPoint): [number, number] {
  return [box.left + ((offset + 100) / 200) * box.width, box.top + (row / 100) * box.height];
}

/** The point a press at client point (`x`, `y`) grabs (within `GRAB` pixels), or -1. */
export function grabbedPoint(points: ShearPoint[], box: Box, x: number, y: number): number {
  let best = -1;
  let closest = GRAB;
  points.forEach((p, i) => {
    const [px, py] = clientPoint(box, p);
    const d = Math.hypot(px - x, py - y);
    if (d <= closest) [best, closest] = [i, d];
  });
  return best;
}

/** Whether client point (`x`, `y`) is far enough off the grid to drag a point off it. */
export function offGrid(box: Box, x: number, y: number): boolean {
  return (
    x < box.left - OFF_GRID ||
    x > box.left + box.width + OFF_GRID ||
    y < box.top - OFF_GRID ||
    y > box.top + box.height + OFF_GRID
  );
}

/**
 * `points` with a point added at `at` (Photoshop: a click on the grid), and its index; `null`
 * when there are 16 already or `at` is too close to another point's row.
 */
export function withPointAdded(
  points: ShearPoint[],
  at: ShearPoint,
): { points: ShearPoint[]; index: number } | null {
  if (points.length >= SHEAR_POINTS) return null;
  if (points.some(([row]) => Math.abs(row - at[0]) < MIN_GAP)) return null;
  const index = points.findIndex(([row]) => row > at[0]);
  if (index <= 0) return null;
  return { points: [...points.slice(0, index), at, ...points.slice(index)], index };
}

/**
 * `points` with point `index` moved to `at`: the first and last stay on the top and bottom
 * rows, the others between their neighbours' rows.
 */
export function withPointMoved(points: ShearPoint[], index: number, at: ShearPoint): ShearPoint[] {
  const last = points.length - 1;
  const offset = Math.min(Math.max(at[1], -100), 100);
  const row =
    index === 0
      ? 0
      : index === last
        ? 100
        : Math.min(Math.max(at[0], points[index - 1][0] + MIN_GAP), points[index + 1][0] - MIN_GAP);
  return points.map((p, i) => (i === index ? [row, offset] : p));
}

/** `points` without point `index`; the first and last cannot be removed. */
export function withPointRemoved(points: ShearPoint[], index: number): ShearPoint[] {
  if (index <= 0 || index >= points.length - 1) return points;
  return points.filter((_, i) => i !== index);
}

/**
 * `points` after key `key` (as `KeyboardEvent.key`) on point `index`: the arrows move it by 1 %
 * (10 % with Shift), Delete or Backspace removes it; `null` for another key.
 */
export function withPointKey(
  points: ShearPoint[],
  index: number,
  key: string,
  shift: boolean,
): ShearPoint[] | null {
  if (key === "Delete" || key === "Backspace") return withPointRemoved(points, index);
  const step = shift ? 10 : 1;
  const move: Record<string, [number, number]> = {
    ArrowLeft: [0, -step],
    ArrowRight: [0, step],
    ArrowUp: [-step, 0],
    ArrowDown: [step, 0],
  };
  const d = move[key];
  if (!d) return null;
  const [row, offset] = points[index];
  return withPointMoved(points, index, [row + d[0], offset + d[1]]);
}

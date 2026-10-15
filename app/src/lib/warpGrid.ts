// Edit > Transform > Warp's grid edited on the canvas (ADR 0044), as in Photoshop: its anchors
// and their Bézier handles dragged, a patch bent by dragging inside it, the grid's size changed,
// split where clicked. A grid is `columns × rows` bicubic Bézier patches whose control points
// (content pixels, row-major, `3·columns + 1` per row and `3·rows + 1` rows) are those of
// slopshop_core::warp::Warp; the engine validates and draws what is sent.
//
// The overlay shows anchors (the patches' corners) and handles (the other points on their
// edges); the points inside a patch follow its edges as a Coons patch does, so that moving an
// edge bends the patch smoothly.

import * as homography from "./homography";
import type { Homography } from "./homography";

export type Point = [number, number];

/** A warp grid: its patches and control points (see above). */
export type Grid = { columns: number; rows: number; points: Point[] };

/** Most patches per side (slopshop_core::warp::MAX_PATCHES). */
export const MAX_PATCHES = 16;

/** Photoshop's grid sizes in the options bar: Default (3 × 3), 4 × 4, 5 × 5. */
export const GRID_SIZES: readonly number[] = [3, 4, 5];

/** Control points per row. */
export function stride(grid: Grid): number {
  return 3 * grid.columns + 1;
}

/** The cubic Bernstein polynomials at `t`. */
function bernstein(t: number): [number, number, number, number] {
  const s = 1 - t;
  return [s * s * s, 3 * s * s * t, 3 * s * t * t, t * t * t];
}

/** The patch holding parameter `p` (`0..=patches`, clamped) and the parameter within it. */
function patchOf(p: number, patches: number): [number, number] {
  const k = Math.min(Math.max(Math.floor(p), 0), patches - 1);
  return [k, p - k];
}

/** Where parameter (`s`, `t`) (`0..=columns`, `0..=rows`) lands, in content pixels. */
export function evaluate(grid: Grid, s: number, t: number): Point {
  const [pc, u] = patchOf(s, grid.columns);
  const [pr, v] = patchOf(t, grid.rows);
  const [bu, bv] = [bernstein(u), bernstein(v)];
  const n = stride(grid);
  let x = 0;
  let y = 0;
  for (let j = 0; j < 4; j++) {
    for (let i = 0; i < 4; i++) {
      const [px, py] = grid.points[(3 * pr + j) * n + 3 * pc + i];
      const w = bu[i] * bv[j];
      x += w * px;
      y += w * py;
    }
  }
  return [x, y];
}

/** The plain grid of `columns × rows` patches over `frame` (`[x0, y0, x1, y1]`). */
export function identityGrid(frame: number[], columns: number, rows: number): Grid {
  const [x0, y0, x1, y1] = frame;
  const [nx, ny] = [3 * columns + 1, 3 * rows + 1];
  const points: Point[] = [];
  for (let j = 0; j < ny; j++) {
    for (let i = 0; i < nx; i++) {
      points.push([x0 + ((x1 - x0) * i) / (nx - 1), y0 + ((y1 - y0) * j) / (ny - 1)]);
    }
  }
  return { columns, rows, points };
}

/** The grid's lines (the patches' edges), each sampled `samples` times: rows, then columns. */
export function gridLines(grid: Grid, samples = 24): Point[][] {
  const lines: Point[][] = [];
  const along = (f: (k: number) => Point) =>
    Array.from({ length: samples + 1 }, (_, k) => f(k / samples));
  for (let r = 0; r <= grid.rows; r++)
    lines.push(along((k) => evaluate(grid, grid.columns * k, r)));
  for (let c = 0; c <= grid.columns; c++)
    lines.push(along((k) => evaluate(grid, c, grid.rows * k)));
  return lines;
}

/** What point `index` is: a patch's corner, a handle on an edge, or inside a patch. */
export function pointKind(grid: Grid, index: number): "anchor" | "handle" | "inner" {
  const n = stride(grid);
  const [i, j] = [index % n, Math.floor(index / n)];
  if (i % 3 === 0 && j % 3 === 0) return "anchor";
  return i % 3 === 0 || j % 3 === 0 ? "handle" : "inner";
}

/** Each handle with the anchor it hangs from (the overlay draws a line between them). */
export function handles(grid: Grid): { handle: number; anchor: number }[] {
  const n = stride(grid);
  const rows = 3 * grid.rows + 1;
  const out: { handle: number; anchor: number }[] = [];
  for (let j = 0; j < rows; j++) {
    for (let i = 0; i < n; i++) {
      if (pointKind(grid, j * n + i) !== "handle") continue;
      // The nearer anchor along its edge: 1 hangs from 0, 2 from 3.
      const anchor =
        j % 3 === 0 ? j * n + (i % 3 === 1 ? i - 1 : i + 1) : (j % 3 === 1 ? j - 1 : j + 1) * n + i;
      out.push({ handle: j * n + i, anchor });
    }
  }
  return out;
}

/** The anchors' indices. */
export function anchors(grid: Grid): number[] {
  const n = stride(grid);
  const out: number[] = [];
  for (let j = 0; j <= 3 * grid.rows; j += 3) {
    for (let i = 0; i < n; i += 3) out.push(j * n + i);
  }
  return out;
}

/**
 * `grid` with points on the patches' edges moved by `deltas` (index → move); the points inside
 * each patch touched follow as a Coons patch of its edges (bilinearly blended): each moves by
 * `(1 − j/3)·Δ(i,0) + (j/3)·Δ(i,3) + (1 − i/3)·Δ(0,j) + (i/3)·Δ(3,j)` minus the corners'
 * bilinear blend.
 */
export function moveEdgePoints(grid: Grid, deltas: Map<number, Point>): Grid {
  const n = stride(grid);
  const points = grid.points.map((p) => [p[0], p[1]] as Point);
  const patches = new Set<number>();
  for (const [index, [dx, dy]] of deltas) {
    points[index] = [points[index][0] + dx, points[index][1] + dy];
    const [i, j] = [index % n, Math.floor(index / n)];
    // The patches whose edges hold this point.
    for (const pc of [Math.floor(i / 3), Math.ceil(i / 3) - 1]) {
      for (const pr of [Math.floor(j / 3), Math.ceil(j / 3) - 1]) {
        if (pc >= 0 && pc < grid.columns && pr >= 0 && pr < grid.rows) {
          patches.add(pr * grid.columns + pc);
        }
      }
    }
  }
  const delta = (index: number): Point => deltas.get(index) ?? [0, 0];
  for (const patch of patches) {
    const [pc, pr] = [patch % grid.columns, Math.floor(patch / grid.columns)];
    const at = (i: number, j: number) => (3 * pr + j) * n + 3 * pc + i;
    for (let j = 1; j <= 2; j++) {
      for (let i = 1; i <= 2; i++) {
        const [fu, fv] = [i / 3, j / 3];
        const terms: [number, number][] = [
          [at(i, 0), 1 - fv],
          [at(i, 3), fv],
          [at(0, j), 1 - fu],
          [at(3, j), fu],
          [at(0, 0), -(1 - fu) * (1 - fv)],
          [at(3, 0), -fu * (1 - fv)],
          [at(0, 3), -(1 - fu) * fv],
          [at(3, 3), -fu * fv],
        ];
        let [dx, dy] = [0, 0];
        for (const [k, w] of terms) {
          const [ddx, ddy] = delta(k);
          dx += w * ddx;
          dy += w * ddy;
        }
        const p = points[at(i, j)];
        points[at(i, j)] = [p[0] + dx, p[1] + dy];
      }
    }
  }
  return { ...grid, points };
}

/** `grid` with anchor `index` moved by (`dx`, `dy`), its handles with it (Photoshop's drag). */
export function moveAnchor(grid: Grid, index: number, dx: number, dy: number): Grid {
  const n = stride(grid);
  const [i, j] = [index % n, Math.floor(index / n)];
  const deltas = new Map<number, Point>([[index, [dx, dy]]]);
  for (const [di, dj] of [
    [-1, 0],
    [1, 0],
    [0, -1],
    [0, 1],
  ]) {
    const [hi, hj] = [i + di, j + dj];
    if (hi >= 0 && hi < n && hj >= 0 && hj <= 3 * grid.rows) deltas.set(hj * n + hi, [dx, dy]);
  }
  return moveEdgePoints(grid, deltas);
}

/** `grid` with handle `index` moved by (`dx`, `dy`). */
export function moveHandle(grid: Grid, index: number, dx: number, dy: number): Grid {
  return moveEdgePoints(grid, new Map([[index, [dx, dy] as Point]]));
}

/**
 * `grid` bent so that the point at parameter (`s`, `t`) moves by (`dx`, `dy`), as dragging inside
 * a patch does in Photoshop: the patch's corners stay, its handles move (the least they can, each
 * by its weight at that point), the points inside following.
 */
export function bendPatch(grid: Grid, s: number, t: number, dx: number, dy: number): Grid {
  const [pc, u] = patchOf(s, grid.columns);
  const [pr, v] = patchOf(t, grid.rows);
  const [bu, bv] = [bernstein(u), bernstein(v)];
  const n = stride(grid);
  const at = (i: number, j: number) => (3 * pr + j) * n + 3 * pc + i;
  // The surface point's weight of each handle: its own, and through the points inside.
  const weights = new Map<number, number>();
  const add = (k: number, w: number) => weights.set(k, (weights.get(k) ?? 0) + w);
  for (let j = 0; j < 4; j++) {
    for (let i = 0; i < 4; i++) {
      const w = bu[i] * bv[j];
      const edgeI = i === 0 || i === 3;
      const edgeJ = j === 0 || j === 3;
      if (edgeI && edgeJ) continue; // a corner: it stays
      if (edgeI || edgeJ) {
        add(at(i, j), w);
      } else {
        // Inside: it follows the handles in its row and column (see moveEdgePoints).
        add(at(i, 0), w * (1 - j / 3));
        add(at(i, 3), w * (j / 3));
        add(at(0, j), w * (1 - i / 3));
        add(at(3, j), w * (i / 3));
      }
    }
  }
  let norm = 0;
  for (const w of weights.values()) norm += w * w;
  if (norm < 1e-12) return grid;
  const deltas = new Map<number, Point>();
  for (const [k, w] of weights) deltas.set(k, [(w * dx) / norm, (w * dy) / norm]);
  return moveEdgePoints(grid, deltas);
}

/**
 * The grid of `columns × rows` patches closest to `grid`'s shape: each new patch interpolates
 * the old surface at 4 × 4 points (exact when the new lines fall on the old surface's cubic
 * pieces, e.g. a plain grid, or 3 × 3 to 6 × 6).
 */
export function regrid(grid: Grid, columns: number, rows: number): Grid {
  // Control points interpolating a cubic at parameters 0, 1/3, 2/3, 1.
  const inverse = [
    [1, 0, 0, 0],
    [-5 / 6, 3, -3 / 2, 1 / 3],
    [1 / 3, -3 / 2, 3, -5 / 6],
    [0, 0, 0, 1],
  ];
  const [sx, sy] = [grid.columns / columns, grid.rows / rows];
  const points: Point[] = [];
  for (let gj = 0; gj <= 3 * rows; gj++) {
    const pr = Math.min(Math.floor(gj / 3), rows - 1);
    const j = gj - 3 * pr;
    for (let gi = 0; gi <= 3 * columns; gi++) {
      const pc = Math.min(Math.floor(gi / 3), columns - 1);
      const i = gi - 3 * pc;
      let [x, y] = [0, 0];
      for (let l = 0; l < 4; l++) {
        const wl = inverse[j][l];
        if (wl === 0) continue;
        for (let k = 0; k < 4; k++) {
          const w = inverse[i][k] * wl;
          if (w === 0) continue;
          const [px, py] = evaluate(grid, (pc + k / 3) * sx, (pr + l / 3) * sy);
          x += w * px;
          y += w * py;
        }
      }
      points.push([x, y]);
    }
  }
  return { columns, rows, points };
}

/** The four points of a cubic split at `u` (de Casteljau): the first piece, then the second. */
function splitCubic(p: Point[], u: number): [Point[], Point[]] {
  const lerp = (a: Point, b: Point): Point => [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u];
  const [a, b, c] = [lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3])];
  const [d, e] = [lerp(a, b), lerp(b, c)];
  const f = lerp(d, e);
  return [
    [p[0], a, d, f],
    [f, e, c, p[3]],
  ];
}

/** Splits closer than this to an existing line (in patch parameters) add nothing. */
const MIN_SPLIT = 0.02;

/**
 * `grid` with a new column of patches at parameter `s` (Split Warp Vertically): the patches it
 * crosses cut in two, the shape unchanged. `null` where it cannot (on a line, or at the most
 * patches).
 */
export function splitColumns(grid: Grid, s: number): Grid | null {
  const [pc, u] = patchOf(s, grid.columns);
  if (grid.columns >= MAX_PATCHES || u < MIN_SPLIT || u > 1 - MIN_SPLIT) return null;
  const n = stride(grid);
  const points: Point[] = [];
  for (let j = 0; j <= 3 * grid.rows; j++) {
    const row = grid.points.slice(j * n, (j + 1) * n);
    const [first, second] = splitCubic(row.slice(3 * pc, 3 * pc + 4), u);
    points.push(...row.slice(0, 3 * pc), ...first, ...second.slice(1), ...row.slice(3 * pc + 4));
  }
  return { columns: grid.columns + 1, rows: grid.rows, points };
}

/** `grid` with its rows and columns swapped. */
function transpose(grid: Grid): Grid {
  const n = stride(grid);
  const m = 3 * grid.rows + 1;
  const points: Point[] = [];
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < m; j++) points.push(grid.points[j * n + i]);
  }
  return { columns: grid.rows, rows: grid.columns, points };
}

/** `grid` with a new row of patches at parameter `t` (Split Warp Horizontally); see above. */
export function splitRows(grid: Grid, t: number): Grid | null {
  const split = splitColumns(transpose(grid), t);
  return split && transpose(split);
}

/** How Split Warp cuts the grid where clicked (Photoshop's three). */
export type SplitMode = "crosswise" | "vertical" | "horizontal";

/** `grid` split at parameter (`s`, `t`) by `mode`; `null` when nothing would be added. */
export function split(grid: Grid, mode: SplitMode, s: number, t: number): Grid | null {
  if (mode === "vertical") return splitColumns(grid, s);
  if (mode === "horizontal") return splitRows(grid, t);
  const across = splitColumns(grid, s) ?? grid;
  const down = splitRows(across, t);
  if (down) return down;
  return across === grid ? null : across;
}

/**
 * The grid's parameter (`s`, `t`) under document point (`x`, `y`) of a grid placed by
 * `toDocument`, found on its mesh (each patch cut into `cells × cells`): where the grid folds,
 * the last piece wins, as the engine draws it. `null` outside the grid.
 */
export function parameterAt(
  grid: Grid,
  toDocument: Homography,
  x: number,
  y: number,
  cells = 12,
): Point | null {
  const [ns, nt] = [grid.columns * cells, grid.rows * cells];
  const vertex: Point[] = [];
  for (let b = 0; b <= nt; b++) {
    for (let a = 0; a <= ns; a++) {
      const [cx, cy] = evaluate(grid, a / cells, b / cells);
      vertex.push(homography.apply(toDocument, cx, cy));
    }
  }
  let found: Point | null = null;
  const v = (a: number, b: number) => vertex[b * (ns + 1) + a];
  for (let b = 0; b < nt; b++) {
    for (let a = 0; a < ns; a++) {
      const corners: [Point, Point][] = [
        [v(a, b), [a, b]],
        [v(a + 1, b), [a + 1, b]],
        [v(a + 1, b + 1), [a + 1, b + 1]],
        [v(a, b + 1), [a, b + 1]],
      ];
      for (const tri of [
        [corners[0], corners[1], corners[2]],
        [corners[0], corners[2], corners[3]],
      ]) {
        const hit = barycentric(
          tri.map((c) => c[0]),
          x,
          y,
        );
        if (!hit) continue;
        let [ps, pt] = [0, 0];
        tri.forEach(([, [ga, gb]], k) => {
          ps += (hit[k] * ga) / cells;
          pt += (hit[k] * gb) / cells;
        });
        found = [ps, pt];
      }
    }
  }
  return found;
}

/** The barycentric weights of (`x`, `y`) in triangle `p`, or null outside it (edges inside). */
function barycentric(p: Point[], x: number, y: number): [number, number, number] | null {
  const [[ax, ay], [bx, by], [cx, cy]] = p;
  const det = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
  if (Math.abs(det) < 1e-12) return null;
  const wb = ((x - ax) * (cy - ay) - (cx - ax) * (y - ay)) / det;
  const wc = ((bx - ax) * (y - ay) - (x - ax) * (by - ay)) / det;
  const wa = 1 - wb - wc;
  const eps = -1e-9;
  return wa >= eps && wb >= eps && wc >= eps ? [wa, wb, wc] : null;
}

/** Whether two grids are the same (same patches, same points). */
export function sameGrid(a: Grid, b: Grid): boolean {
  return (
    a.columns === b.columns &&
    a.rows === b.rows &&
    a.points.length === b.points.length &&
    a.points.every(([x, y], k) => x === b.points[k][0] && y === b.points[k][1])
  );
}

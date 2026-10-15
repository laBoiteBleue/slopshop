import { test } from "vitest";
import assert from "node:assert/strict";
import type { Homography } from "../src/lib/homography";
import {
  anchors,
  bendPatch,
  evaluate,
  gridLines,
  handles,
  identityGrid,
  MAX_PATCHES,
  moveAnchor,
  moveEdgePoints,
  moveHandle,
  parameterAt,
  pointKind,
  regrid,
  sameGrid,
  split,
  splitColumns,
  splitRows,
  stride,
  type Grid,
  type Point,
} from "../src/lib/warpGrid";

const FRAME = [10, 20, 70, 50];
const IDENTITY: Homography = [1, 0, 0, 1, 0, 0, 0, 0, 1];

function close(a: Point, b: Point, tolerance = 1e-9) {
  assert.ok(
    Math.abs(a[0] - b[0]) <= tolerance && Math.abs(a[1] - b[1]) <= tolerance,
    `${a} is not ${b}`,
  );
}

/** A grid bent like a style: an arch over the frame. */
function arched(): Grid {
  const plain = identityGrid(FRAME, 3, 3);
  return {
    ...plain,
    points: plain.points.map(([x, y]) => [x, y - 12 * Math.sin((Math.PI * (x - 10)) / 60)]),
  };
}

test("a plain grid places the frame as it is", () => {
  const grid = identityGrid(FRAME, 3, 2);
  assert.equal(grid.points.length, 10 * 7);
  close(evaluate(grid, 0, 0), [10, 20]);
  close(evaluate(grid, 3, 2), [70, 50]);
  close(evaluate(grid, 1.5, 0.5), [40, 27.5]);
  // Beyond the grid, the border patches are extrapolated.
  close(evaluate(grid, 3.5, 2), [80, 50]);
});

test("the grid's lines: its rows then its columns, along its edges", () => {
  const lines = gridLines(identityGrid(FRAME, 3, 2), 4);
  assert.equal(lines.length, 3 + 4);
  assert.equal(lines[0].length, 5);
  close(lines[0][0], [10, 20]);
  close(lines[0][4], [70, 20]);
  close(lines[3][4], [10, 50]);
});

test("anchors are the patches' corners, handles the other points on their edges", () => {
  const grid = identityGrid(FRAME, 3, 3);
  assert.equal(anchors(grid).length, 16);
  const list = handles(grid);
  assert.equal(list.length, 48);
  const n = stride(grid);
  // On the top edge: 1 hangs from 0, 2 from 3; down the left edge: row 1 from row 0.
  assert.deepEqual(list[0], { handle: 1, anchor: 0 });
  assert.deepEqual(list[1], { handle: 2, anchor: 3 });
  assert.ok(list.some((h) => h.handle === n && h.anchor === 0));
  assert.ok(list.some((h) => h.handle === 2 * n && h.anchor === 3 * n));
  assert.equal(pointKind(grid, 0), "anchor");
  assert.equal(pointKind(grid, 1), "handle");
  assert.equal(pointKind(grid, n + 1), "inner");
});

test("the points inside a patch follow its edges: an affine move of the edges moves them alike", () => {
  const grid = identityGrid(FRAME, 2, 2);
  const n = stride(grid);
  const map = ([x, y]: Point): Point => [0.1 * x - 0.2 * y + 3, 0.05 * x + 0.3 * y - 1];
  const deltas = new Map<number, Point>();
  grid.points.forEach((p, k) => {
    const [i, j] = [k % n, Math.floor(k / n)];
    if (i % 3 === 0 || j % 3 === 0) deltas.set(k, map(p));
  });
  const moved = moveEdgePoints(grid, deltas);
  moved.points.forEach((p, k) => {
    const [dx, dy] = map(grid.points[k]);
    close(p, [grid.points[k][0] + dx, grid.points[k][1] + dy]);
  });
});

test("an anchor drags its handles; the far side of the grid stays", () => {
  const grid = identityGrid(FRAME, 3, 3);
  const n = stride(grid);
  const anchor = 3 * n + 3;
  const moved = moveAnchor(grid, anchor, 5, -2);
  for (const k of [anchor, anchor - 1, anchor + 1, anchor - n, anchor + n]) {
    close(moved.points[k], [grid.points[k][0] + 5, grid.points[k][1] - 2]);
  }
  close(evaluate(moved, 1, 1), [35, 28]);
  // Points of patches away from it do not move.
  close(moved.points[n * n - 1], grid.points[n * n - 1]);
  close(moved.points[7 * n + 7], grid.points[7 * n + 7]);
  // A corner of the whole grid has two handles.
  const corner = moveAnchor(grid, 0, 1, 1);
  close(corner.points[1], [grid.points[1][0] + 1, grid.points[1][1] + 1]);
  close(corner.points[n], [grid.points[n][0] + 1, grid.points[n][1] + 1]);
});

test("a handle moves alone among the edges; the patches it borders bend", () => {
  const grid = identityGrid(FRAME, 3, 3);
  const n = stride(grid);
  const handle = 3 * n + 4; // on the line between the first two rows of patches
  const moved = moveHandle(grid, handle, 0, 6);
  moved.points.forEach((p, k) => {
    if (pointKind(grid, k) !== "inner") {
      close(p, k === handle ? [grid.points[k][0], grid.points[k][1] + 6] : grid.points[k]);
    }
  });
  // The patches above and below it bend; the others do not.
  assert.notDeepEqual(moved.points[n + 4], grid.points[n + 4]);
  assert.notDeepEqual(moved.points[4 * n + 4], grid.points[4 * n + 4]);
  close(moved.points[n + 1], grid.points[n + 1]);
  close(moved.points[4 * n + 7], grid.points[4 * n + 7]);
});

test("dragging inside a patch moves the point under the pointer exactly, its corners stay", () => {
  const grid = arched();
  const [s, t] = [1.3, 0.6];
  const before = evaluate(grid, s, t);
  const bent = bendPatch(grid, s, t, 4, -7);
  close(evaluate(bent, s, t), [before[0] + 4, before[1] - 7]);
  for (const k of anchors(grid)) close(bent.points[k], grid.points[k]);
  // On a corner, nothing can move but the anchor: unchanged.
  assert.equal(bendPatch(grid, 1, 1, 4, 4), grid);
  // On an edge, the edge bends (its handles move).
  const edge = bendPatch(grid, 1.5, 1, 0, 5);
  close(evaluate(edge, 1.5, 1), [evaluate(grid, 1.5, 1)[0], evaluate(grid, 1.5, 1)[1] + 5]);
});

test("a new grid size keeps the shape: exact for a plain grid and for whole subdivisions", () => {
  const plain = regrid(identityGrid(FRAME, 3, 3), 5, 4);
  const expected = identityGrid(FRAME, 5, 4);
  assert.equal(plain.points.length, expected.points.length);
  plain.points.forEach((p, k) => close(p, expected.points[k]));
  const grid = arched();
  const finer = regrid(grid, 6, 6);
  for (const [s, t] of [
    [0.2, 0.3],
    [1.7, 2.9],
    [3, 3],
  ]) {
    close(evaluate(finer, s * 2, t * 2), evaluate(grid, s, t), 1e-9);
  }
  // Coarser or uneven: close to the shape.
  const coarse = regrid(grid, 4, 4);
  close(evaluate(coarse, 2, 2), evaluate(grid, 1.5, 1.5), 0.5);
});

test("a split cuts the patches it crosses, the shape unchanged", () => {
  const grid = arched();
  const across = splitColumns(grid, 1.25);
  assert.ok(across);
  assert.equal(across.columns, 4);
  assert.equal(across.rows, 3);
  // Patch 1 became two: [1, 1.25] and [1.25, 2] of the old parameters.
  close(evaluate(across, 1.5, 0.4), evaluate(grid, 1.125, 0.4));
  close(evaluate(across, 2.5, 2.2), evaluate(grid, 1.625, 2.2));
  close(evaluate(across, 3.5, 1), evaluate(grid, 2.5, 1));
  const down = splitRows(grid, 2.5);
  assert.ok(down);
  assert.equal(down.rows, 4);
  close(evaluate(down, 0.7, 2.5), evaluate(grid, 0.7, 2.25));
  close(evaluate(down, 0.7, 3.5), evaluate(grid, 0.7, 2.75));
  // On a line, or at the most patches: nothing to add.
  assert.equal(splitColumns(grid, 2), null);
  assert.equal(splitColumns(identityGrid(FRAME, MAX_PATCHES, 1), 0.5), null);
});

test("Split Warp crosswise adds a row and a column; one alone on a line", () => {
  const grid = identityGrid(FRAME, 3, 3);
  assert.deepEqual(
    [split(grid, "crosswise", 1.5, 1.5)].map((g) => g && [g.columns, g.rows]),
    [[4, 4]],
  );
  const cross = split(grid, "crosswise", 1.5, 1);
  assert.deepEqual(cross && [cross.columns, cross.rows], [4, 3]);
  const vertical = split(grid, "vertical", 1.5, 1.5);
  assert.deepEqual(vertical && [vertical.columns, vertical.rows], [4, 3]);
  const horizontal = split(grid, "horizontal", 1.5, 1.5);
  assert.deepEqual(horizontal && [horizontal.columns, horizontal.rows], [3, 4]);
  assert.equal(split(grid, "crosswise", 1, 2), null);
});

test("the parameter under a document point is found on the placed grid", () => {
  const plain = identityGrid(FRAME, 3, 3);
  const moved: Homography = [2, 0, 0, 2, 5, 5, 0, 0, 1];
  const at = parameterAt(plain, moved, 2 * 40 + 5, 2 * 35 + 5);
  assert.ok(at);
  close(at, [1.5, 1.5], 1e-9);
  assert.equal(parameterAt(plain, IDENTITY, 5, 5), null);
  const grid = arched();
  const [x, y] = evaluate(grid, 2.2, 0.7);
  const found = parameterAt(grid, IDENTITY, x, y);
  assert.ok(found);
  close(found, [2.2, 0.7], 0.02);
});

test("grids are the same when their patches and points are", () => {
  const grid = identityGrid(FRAME, 3, 3);
  assert.ok(sameGrid(grid, identityGrid(FRAME, 3, 3)));
  assert.ok(!sameGrid(grid, identityGrid(FRAME, 4, 3)));
  assert.ok(!sameGrid(grid, moveAnchor(grid, 0, 1, 0)));
});

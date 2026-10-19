import { test } from "vitest";
import assert from "node:assert/strict";
import {
  addPlane,
  isValid,
  joinCorner,
  MAX_PLANES,
  moveCorner,
  NO_PLANES,
  planeAt,
  planeLines,
  planesRequest,
  removePlane,
  samePlanes,
  type Planes,
  type Point,
} from "../src/lib/perspectiveWarp";

/** One plane over (10, 10)–(50, 40). */
function one(): Planes {
  const planes = addPlane(NO_PLANES, [10, 10], [50, 40], 3);
  assert.ok(planes);
  return planes;
}

test("a drag draws a plane, its corners in order around it, unwarped", () => {
  const planes = addPlane(NO_PLANES, [50, 40], [10, 10], 3);
  assert.deepEqual(planes, {
    layout: [
      [10, 10],
      [50, 10],
      [50, 40],
      [10, 40],
    ],
    warped: [
      [10, 10],
      [50, 10],
      [50, 40],
      [10, 40],
    ],
    planes: [[0, 1, 2, 3]],
  });
  // Too small: nothing.
  assert.equal(addPlane(NO_PLANES, [10, 10], [12, 40], 3), null);
});

test("a plane drawn next to another snaps to its corners: they share the edge", () => {
  // Starting by the first plane's top right corner, ending by its bottom right's height.
  const planes = addPlane(one(), [51, 11], [90, 39], 3);
  assert.ok(planes);
  assert.equal(planes.layout.length, 6);
  assert.deepEqual(planes.planes[1], [1, 4, 5, 2]);
  assert.deepEqual(planes.layout[4], [90, 11]);
  // Out of reach: four corners of its own.
  const apart = addPlane(one(), [60, 11], [90, 39], 3);
  assert.equal(apart?.layout.length, 8);
});

test("no more than the most planes", () => {
  let planes = NO_PLANES;
  for (let k = 0; k < MAX_PLANES; k++) {
    const next = addPlane(planes, [k * 10, 0], [k * 10 + 8, 8], 0);
    assert.ok(next);
    planes = next;
  }
  assert.equal(addPlane(planes, [0, 20], [10, 30], 0), null);
});

test("Layout mode lays a corner out and its warp follows; Warp mode moves it", () => {
  const warped = moveCorner(one(), 0, [14, 12], "warp");
  assert.ok(warped);
  assert.deepEqual(warped.warped[0], [14, 12]);
  assert.deepEqual(warped.layout[0], [10, 10]);
  const laid = moveCorner(warped, 0, [8, 9], "layout");
  assert.ok(laid);
  assert.deepEqual(laid.layout[0], [8, 9]);
  // Moved by as much: (−2, −1).
  assert.deepEqual(laid.warped[0], [12, 11]);
});

test("a corner dragged across its plane is refused: the plane would fold", () => {
  assert.equal(moveCorner(one(), 0, [60, 50], "warp"), null);
  assert.equal(moveCorner(one(), 0, [45, 35], "layout"), null);
});

test("a shared corner moves both planes", () => {
  const two = addPlane(one(), [50, 10], [90, 40], 3);
  assert.ok(two);
  const moved = moveCorner(two, 1, [55, 5], "warp");
  assert.ok(moved);
  assert.deepEqual(moved.warped[moved.planes[0][1]], [55, 5]);
  assert.deepEqual(moved.warped[moved.planes[1][0]], [55, 5]);
});

test("a corner dropped by another plane's corner joins it", () => {
  const apart = addPlane(one(), [60, 10], [90, 40], 3);
  assert.ok(apart);
  // The second plane's top left corner (index 4) laid out by the first plane's top right.
  const near = moveCorner(apart, 4, [51, 11], "layout");
  assert.ok(near);
  const joined = joinCorner(near, 4, 3);
  assert.equal(joined.layout.length, 7);
  assert.deepEqual(joined.planes[1][0], 1);
  assert.ok(isValid(joined));
  // Nothing within reach: unchanged.
  assert.equal(joinCorner(apart, 4, 3), apart);
});

test("removing a plane drops the corners only it used", () => {
  const two = addPlane(one(), [50, 10], [90, 40], 3);
  assert.ok(two);
  const left = removePlane(two, 1);
  assert.deepEqual(left.planes, [[0, 1, 2, 3]]);
  assert.equal(left.layout.length, 4);
  const right = removePlane(two, 0);
  assert.deepEqual(right.planes, [[0, 2, 3, 1]]);
  assert.deepEqual(right.layout, [
    [50, 10],
    [50, 40],
    [90, 10],
    [90, 40],
  ]);
});

test("the plane under a point, the last one on top", () => {
  const two = addPlane(one(), [50, 10], [90, 40], 3);
  assert.ok(two);
  assert.equal(planeAt(two, "layout", [20, 20]), 0);
  assert.equal(planeAt(two, "layout", [70, 20]), 1);
  // On the shared edge: the plane drawn last.
  assert.equal(planeAt(two, "layout", [50, 20]), 1);
  assert.equal(planeAt(two, "layout", [5, 5]), null);
});

test("a plane's lines: its edges, then lines across as its homography places them", () => {
  const planes = moveCorner(one(), 1, [50, 0], "warp");
  assert.ok(planes);
  const lines = planeLines(planes, 0, "warp", 2);
  assert.equal(lines.length, 6);
  assert.deepEqual(lines[0], [
    [10, 10],
    [50, 0],
  ]);
  // Across, halfway as laid out: on the top edge as moved, nearer its far (stretched) end.
  const [top] = lines[4];
  const near = (p: Point, q: Point) => Math.abs(p[0] - q[0]) < 1e-9 && Math.abs(p[1] - q[1]) < 1e-9;
  assert.ok(!near(top, [30, 5]));
  const along = (top[0] - 10) / 40;
  assert.ok(Math.abs(top[1] - (10 - 10 * along)) < 1e-9);
  // In Layout mode, straight across the quad as laid out.
  assert.deepEqual(planeLines(planes, 0, "layout", 2)[4], [
    [30, 10],
    [30, 40],
  ]);
});

test("the request: planes, or none when there is none", () => {
  const planes = one();
  assert.deepEqual(planesRequest(7, planes), {
    kind: "perspectiveWarpLayer",
    id: 7,
    planes: { layout: planes.layout, warped: planes.warped, planes: planes.planes },
  });
  assert.deepEqual(planesRequest(7, null), { kind: "perspectiveWarpLayer", id: 7 });
  assert.deepEqual(planesRequest(7, NO_PLANES), { kind: "perspectiveWarpLayer", id: 7 });
  assert.ok(samePlanes(planes, one()));
  assert.ok(!samePlanes(planes, moveCorner(planes, 2, [52, 41], "warp") as Planes));
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { IDENTITY, apply, invert, rotation, scaling } from "../src/lib/affine.ts";
import {
  MIN_SCALE,
  boundsUnder,
  boxFrame,
  movedBy,
  resizeCursor,
  rotatedTo,
  scalePercent,
  scaledTo,
  skewDegrees,
  skewedTo,
  snappedScale,
} from "../src/lib/freeTransform.ts";

/** A box 100 × 50 at the origin, its pivot at the center (50, 25). */
const FRAME = boxFrame({ left: 0, top: 0, right: 100, bottom: 50 }, [50, 25]);
const NO_KEYS = { shift: false, alt: false };
const SHIFT = { shift: true, alt: false };
const ALT = { shift: false, alt: true };

const closeAll = (actual: readonly number[], expected: readonly number[], tolerance = 1e-9) =>
  assert.ok(
    actual.every((v, i) => Math.abs(v - expected[i]) < tolerance),
    `[${actual}] ≠ [${expected}]`,
  );

test("handles go clockwise from the top-left corner, sides between corners", () => {
  assert.deepEqual(FRAME.handles, [
    [0, 0],
    [50, 0],
    [100, 0],
    [100, 25],
    [100, 50],
    [50, 50],
    [0, 50],
    [0, 25],
  ]);
  assert.deepEqual(FRAME.center, [50, 25]);
});

test("a corner scales along the diagonal, or freely with Shift", () => {
  closeAll(apply(scaledTo(FRAME, IDENTITY, 4, [200, 100], NO_KEYS), 100, 50), [200, 100]);
  // Off the diagonal: the pointer projected on it, both axes alike.
  const projected = scaledTo(FRAME, IDENTITY, 4, [200, 60], NO_KEYS);
  closeAll([projected[0], projected[3]], [1.84, 1.84]);
  const free = scaledTo(FRAME, IDENTITY, 4, [200, 60], SHIFT);
  closeAll([free[0], free[3]], [2, 1.2]);
  // The opposite corner stays.
  closeAll(apply(free, 0, 0), [0, 0]);
});

test("a side scales one way, both with Shift", () => {
  const wide = scaledTo(FRAME, IDENTITY, 3, [150, 999], NO_KEYS);
  closeAll([wide[0], wide[3]], [1.5, 1]);
  const both = scaledTo(FRAME, IDENTITY, 3, [150, 999], SHIFT);
  closeAll([both[0], both[3]], [1.5, 1.5]);
  // About the left side's middle.
  closeAll(apply(both, 0, 25), [0, 25]);
});

test("Alt scales about the pivot", () => {
  const m = scaledTo(FRAME, IDENTITY, 4, [150, 75], ALT);
  closeAll(apply(m, 100, 50), [150, 75]);
  closeAll(apply(m, 50, 25), [50, 25]);
  closeAll(apply(m, 0, 0), [-50, -25]);
});

test("a scale never reaches zero", () => {
  const flat = scaledTo(FRAME, IDENTITY, 3, [0, 25], NO_KEYS);
  assert.equal(flat[0], MIN_SCALE);
  assert.ok(invert(flat));
});

test("a scale applies after the transform already there", () => {
  const start = scaling(2, 2);
  // The bottom-right handle is at (200, 100) now; dragging it to (400, 200) doubles again.
  const m = scaledTo(FRAME, start, 4, [400, 200], NO_KEYS);
  closeAll(apply(m, 100, 50), [400, 200]);
  closeAll(apply(m, 0, 0), [0, 0]);
});

test("a side slides along itself to skew, the opposite side stays", () => {
  const m = skewedTo(FRAME, IDENTITY, 1, [60, 0], NO_KEYS);
  closeAll(apply(m, 50, 0), [60, 0]);
  closeAll(apply(m, 50, 50), [50, 50]);
  closeAll([skewDegrees(m)], [(Math.atan2(-0.2, 1) * 180) / Math.PI]);
  // A left or right side slides vertically.
  const v = skewedTo(FRAME, IDENTITY, 3, [100, 35], NO_KEYS);
  closeAll(apply(v, 100, 25), [100, 35]);
  closeAll(apply(v, 0, 25), [0, 25]);
});

test("a rotation turns about the pivot, by steps with Shift, shown in (-180°, 180°]", () => {
  const quarter = rotatedTo(FRAME, IDENTITY, 0, [100, 25], [50, 75], false);
  closeAll([quarter.degrees], [90]);
  closeAll(apply(quarter.matrix, 100, 25), [50, 75]);
  closeAll(apply(quarter.matrix, 50, 25), [50, 25]);
  // 50° snaps to 45°.
  const to = [50 + Math.cos((50 * Math.PI) / 180), 25 + Math.sin((50 * Math.PI) / 180)] as [
    number,
    number,
  ];
  closeAll([rotatedTo(FRAME, IDENTITY, 0, [100, 25], to, true).degrees], [45]);
  // Already upside down (the pivot is now at -50, -25), a quarter more: -90°.
  const start = rotation(Math.PI);
  closeAll([rotatedTo(FRAME, start, Math.PI, [-40, -25], [-50, -15], false).degrees], [-90]);
});

test("a move with Shift keeps to one axis, and snaps on the other axis only", () => {
  assert.deepEqual(movedBy(FRAME, IDENTITY, 10, 3, true, [], 6), { dx: 10, dy: 0, guides: [] });
  assert.deepEqual(movedBy(FRAME, IDENTITY, 2, -7, true, [], 6), { dx: 0, dy: -7, guides: [] });
  const target = { left: 112, top: 2, right: 200, bottom: 52 };
  const snapped = movedBy(FRAME, IDENTITY, 10, 0, true, [target], 6);
  assert.equal(snapped.dx, 12);
  assert.equal(snapped.dy, 0);
  const free = movedBy(FRAME, IDENTITY, 10, 0, false, [target], 6);
  assert.deepEqual([free.dx, free.dy], [12, 2]);
});

test("a dragged handle snaps to an edge, the scale following", () => {
  const target = { left: 203, top: 500, right: 300, bottom: 600 };
  const snapped = snappedScale(FRAME, IDENTITY, 3, [200, 25], NO_KEYS, [target], 6);
  assert.ok(snapped);
  closeAll(apply(snapped.matrix, 100, 25), [203, 25]);
  assert.ok(snapped.guides.length > 0);
  // Nothing near, no targets, or a rotated box: no snap.
  assert.equal(snappedScale(FRAME, IDENTITY, 3, [150, 25], NO_KEYS, [target], 6), null);
  assert.equal(snappedScale(FRAME, IDENTITY, 3, [200, 25], NO_KEYS, [], 6), null);
  assert.equal(snappedScale(FRAME, rotation(0.1), 3, [200, 25], NO_KEYS, [target], 6), null);
});

test("a corner snaps proportionally, by the axis needing the least shift", () => {
  const target = { left: 204, top: 101, right: 300, bottom: 300 };
  const snapped = snappedScale(FRAME, IDENTITY, 4, [200, 100], NO_KEYS, [target], 6);
  assert.ok(snapped);
  // y needed 1 pixel, x 4: the scale is 101 / 50 on both axes.
  closeAll(apply(snapped.matrix, 100, 50), [202, 101]);
});

test("bounds, scale and cursors as shown", () => {
  assert.deepEqual(boundsUnder(FRAME, scaling(-1, 2)), {
    left: -100,
    top: 0,
    right: 0,
    bottom: 100,
  });
  closeAll(Object.values(scalePercent(scaling(2, 3))), [200, 300]);
  closeAll(Object.values(scalePercent(rotation(1))), [100, 100]);
  assert.equal(resizeCursor([10, 0], [0, 0], false), 0);
  assert.equal(resizeCursor([10, 10], [0, 0], false), 1);
  assert.equal(resizeCursor([0, 10], [0, 0], false), 2);
  assert.equal(resizeCursor([-10, 10], [0, 0], false), 3);
  // A skewing side handle: along its side.
  assert.equal(resizeCursor([10, 0], [0, 0], true), 2);
});

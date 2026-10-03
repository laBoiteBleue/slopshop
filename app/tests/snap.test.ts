import { test } from "node:test";
import assert from "node:assert/strict";
import { snapHandle, snapMove } from "../src/lib/snap.ts";

const box = (left: number, top: number, right: number, bottom: number) => ({
  left,
  top,
  right,
  bottom,
});

test("a moved box's edge meets a target's edge within the threshold, with a guide", () => {
  const moved = snapMove(box(0, 0, 100, 100), 97, 0, [box(200, 0, 300, 100)], 6);
  assert.equal(moved.x, 100);
  assert.equal(moved.y, 0);
  assert.deepEqual(moved.guides[0], { x1: 200, x2: 200, y1: 0, y2: 100 });
});

test("a move beyond the threshold stays as it is, without guides", () => {
  const moved = snapMove(box(0, 0, 100, 100), 90, 3, [box(200, 500, 300, 600)], 6);
  assert.deepEqual(moved, { x: 90, y: 3, guides: [] });
});

test("the nearest line wins, and centers snap too", () => {
  const targets = [box(204, 500, 300, 600), box(198, 500, 300, 600)];
  assert.equal(snapMove(box(0, 0, 100, 100), 100, 0, targets, 6).x, 98);
  // The box's center (498) meets the target's (500).
  const centered = snapMove(box(0, 0, 100, 100), 448, 0, [box(0, 0, 1000, 1000)], 6);
  assert.equal(centered.x, 450);
});

test("a dragged handle snaps to a target's edge", () => {
  const snap = snapHandle(100, 0, 1, "x", [box(101, 0, 400, 10)], 6);
  assert.ok(snap);
  assert.equal(snap.shift, 1);
  assert.deepEqual(snap.guides(box(0, 0, 101, 50)), [{ x1: 101, x2: 101, y1: 0, y2: 50 }]);
});

test("a dragged handle snaps to a target's size, shown by two measures", () => {
  // The target is 103 wide: the box (anchored at 0) gets that size.
  const snap = snapHandle(100, 0, 1, "x", [box(300, 0, 403, 10)], 6);
  assert.ok(snap);
  assert.equal(snap.shift, 3);
  const guides = snap.guides(box(0, 0, 103, 50));
  assert.equal(guides.length, 2);
  assert.ok(guides.every((g) => g.measure));
  assert.deepEqual(guides[0], { x1: 0, x2: 103, y1: 25, y2: 25, measure: true });
});

test("a handle scaling about the center, or dragged backwards, matches sizes too", () => {
  // About the center (span 2): the box is twice the handle's distance to the anchor.
  assert.equal(snapHandle(100, 50, 2, "x", [box(500, 0, 604, 10)], 6)?.shift, 2);
  // Dragged up, above its anchor.
  assert.equal(snapHandle(0, 100, 1, "y", [box(0, 500, 10, 603)], 6)?.shift, -3);
});

test("nothing to snap to within the threshold gives null", () => {
  assert.equal(snapHandle(100, 0, 1, "x", [box(300, 0, 500, 10)], 6), null);
  assert.equal(snapHandle(100, 0, 1, "x", [], 6), null);
});

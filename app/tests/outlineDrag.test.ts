import { test } from "vitest";
import assert from "node:assert/strict";
import { outlineOffset } from "../src/lib/outlineDrag";

test("a dragged outline moves by whole document pixels", () => {
  assert.deepEqual(outlineOffset(10.4, -3.6, false), [10, -4]);
  assert.deepEqual(outlineOffset(-0.2, 0.3, false), [0, 0]);
});

test("Shift keeps the outline on a multiple of 45°", () => {
  assert.deepEqual(outlineOffset(30, 4, true), [30, 0]);
  assert.deepEqual(outlineOffset(-3, -40, true), [0, -40]);
  const [dx, dy] = outlineOffset(20, 18, true);
  assert.equal(dx, dy);
  assert.equal(dx, Math.round(Math.hypot(20, 18) / Math.SQRT2));
});

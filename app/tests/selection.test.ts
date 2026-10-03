import { test } from "vitest";
import assert from "node:assert/strict";
import { MAX_BRUSH, modeFromKeys, stepBrush } from "../src/lib/selection";

test("Shift adds, Alt subtracts, both intersect, as in Photoshop", () => {
  assert.equal(modeFromKeys({ shiftKey: true, altKey: false }), "add");
  assert.equal(modeFromKeys({ shiftKey: false, altKey: true }), "subtract");
  assert.equal(modeFromKeys({ shiftKey: true, altKey: true }), "intersect");
  assert.equal(modeFromKeys({ shiftKey: false, altKey: false }), null);
});

test("[ and ] step the brush size, finer when it is small", () => {
  assert.equal(stepBrush(5, true), 6);
  assert.equal(stepBrush(10, true), 15);
  assert.equal(stepBrush(100, false), 90);
  assert.equal(stepBrush(500, true), 600);
  // Off-step sizes land on the step.
  assert.equal(stepBrush(12, true), 15);
});

test("[ undoes ], going down by the step of the range below", () => {
  assert.equal(stepBrush(50, false), 45);
  assert.equal(stepBrush(10, false), 9);
  for (let size = 1; size < 3000; size = stepBrush(size, true)) {
    assert.equal(stepBrush(stepBrush(size, true), false), size, `from ${size}`);
  }
});

test("the brush size stays within 1 and the largest", () => {
  assert.equal(stepBrush(1, false), 1);
  assert.equal(stepBrush(MAX_BRUSH, true), MAX_BRUSH);
});

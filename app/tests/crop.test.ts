import { test } from "vitest";
import assert from "node:assert/strict";
import { aspectRatio, centered, fitRatio, keepRatio } from "../src/lib/crop";

const box = (left: number, top: number, right: number, bottom: number) => ({
  left,
  top,
  right,
  bottom,
});

test("a ratio or a size constrains the frame; free does not", () => {
  assert.equal(aspectRatio({ mode: "free" }), null);
  assert.equal(aspectRatio({ mode: "ratio", width: 16, height: 9 }), 16 / 9);
  assert.equal(aspectRatio({ mode: "size", width: 300, height: 200 }), 1.5);
  assert.equal(aspectRatio({ mode: "ratio", width: 0, height: 9 }), null);
});

test("a ratio chosen fits the largest frame of it in the frame, centered", () => {
  assert.deepEqual(fitRatio(box(0, 0, 400, 300), 1), box(50, 0, 350, 300));
  assert.deepEqual(fitRatio(box(0, 0, 400, 300), 2), box(0, 50, 400, 250));
  assert.deepEqual(centered(box(0, 0, 400, 300), 100, 50), box(150, 125, 250, 175));
});

test("a corner keeps the ratio, the opposite corner staying", () => {
  const start = box(0, 0, 100, 100);
  // The bottom-right corner dragged to (300, 120) at 2:1: the width leads.
  assert.deepEqual(keepRatio(box(0, 0, 300, 120), start, 4, 2), box(0, 0, 300, 150));
  // The top-left corner dragged up, past the height: the height leads.
  assert.deepEqual(keepRatio(box(80, -100, 100, 100), start, 0, 1), box(-100, -100, 100, 100));
  // Drawing from (10, 10) to the left of it.
  assert.deepEqual(keepRatio(box(10, 10, -30, 20), start, null, 2), box(10, 10, -30, 30));
});

test("a side keeps the ratio, the other side centered on the frame as it was", () => {
  const start = box(0, 0, 200, 100);
  assert.deepEqual(keepRatio(box(0, 0, 300, 100), start, 3, 2), box(0, -25, 300, 125));
  assert.deepEqual(keepRatio(box(0, -50, 200, 100), start, 1, 2), box(-50, -50, 250, 100));
});

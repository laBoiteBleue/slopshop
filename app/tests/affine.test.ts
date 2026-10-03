import { test } from "node:test";
import assert from "node:assert/strict";
import {
  IDENTITY,
  about,
  apply,
  invert,
  isIdentity,
  rotation,
  scaling,
  then,
  translation,
} from "../src/lib/affine.ts";

const closePoint = ([x, y]: [number, number], [ex, ey]: [number, number]) =>
  assert.ok(Math.abs(x - ex) < 1e-9 && Math.abs(y - ey) < 1e-9, `(${x}, ${y}) ≠ (${ex}, ${ey})`);

test("`then` applies the first map first", () => {
  closePoint(apply(then(translation(1, 0), scaling(2, 3)), 0, 0), [2, 0]);
  closePoint(apply(then(scaling(2, 3), translation(1, 0)), 0, 0), [1, 0]);
  closePoint(apply(then(scaling(2, 3), translation(1, 0)), 1, 1), [3, 3]);
});

test("a rotation turns clockwise on screen", () => {
  closePoint(apply(rotation(Math.PI / 2), 1, 0), [0, 1]);
});

test("a map about a point keeps that point in place", () => {
  const m = about(then(rotation(0.7), scaling(1.5, 0.5)), 5, 7);
  closePoint(apply(m, 5, 7), [5, 7]);
});

test("the inverse brings points back, and a flat map has none", () => {
  const m = then(then(rotation(0.3), scaling(2, -0.5)), translation(10, -4));
  const inverse = invert(m);
  assert.ok(inverse);
  const [x, y] = apply(m, 3, 8);
  closePoint(apply(inverse, x, y), [3, 8]);
  assert.equal(invert(scaling(0, 1)), null);
  assert.equal(invert([NaN, 0, 0, 1, 0, 0]), null);
});

test("the identity is recognized", () => {
  assert.ok(isIdentity(IDENTITY));
  assert.ok(isIdentity(then(translation(2, 3), translation(-2, -3))));
  assert.ok(!isIdentity(translation(0, 1)));
});

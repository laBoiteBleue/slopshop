import { test } from "node:test";
import assert from "node:assert/strict";
import { compose, decompose, type Map6 } from "../src/lib/transformValues.ts";

function close(actual: number[], expected: number[]) {
  actual.forEach((v, i) => assert.ok(Math.abs(v - expected[i]) < 1e-9, `${actual} ≠ ${expected}`));
}

test("the identity is 100 % where the reference point is", () => {
  const v = decompose([1, 0, 0, 1, 0, 0], [50, 20]);
  close([v.x, v.y, v.width, v.height, v.angle, v.skew], [50, 20, 1, 1, 0, 0]);
});

test("a move, a scale, a rotation and a skew come apart and back together", () => {
  const pivot: [number, number] = [30, 40];
  const v = { x: 120, y: -15, width: 1.5, height: 0.75, angle: 0.4, skew: 0.2 };
  const m = compose(v, pivot);
  const back = decompose(m, pivot);
  close(
    [back.x, back.y, back.width, back.height, back.angle, back.skew],
    [v.x, v.y, v.width, v.height, v.angle, v.skew],
  );
});

test("a flip shows as a negative height", () => {
  const flipped: Map6 = [1, 0, 0, -1, 0, 100];
  const v = decompose(flipped, [0, 50]);
  close([v.width, v.height, v.y], [1, -1, 50]);
  close(compose(v, [0, 50]), flipped);
});

test("a rotation about the reference point keeps it in place", () => {
  const pivot: [number, number] = [10, 10];
  const v = decompose([1, 0, 0, 1, 0, 0], pivot);
  const turned = compose({ ...v, angle: Math.PI / 2 }, pivot);
  // The pivot does not move; (20, 10) turns to (10, 20).
  const at = (x: number, y: number) => [
    turned[0] * x + turned[2] * y + turned[4],
    turned[1] * x + turned[3] * y + turned[5],
  ];
  close(at(10, 10), [10, 10]);
  close(at(20, 10), [10, 20]);
});

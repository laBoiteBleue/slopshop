import { expect, test } from "vitest";
import {
  andThen,
  apply,
  corners,
  fromAffine,
  invert,
  isConvex,
  perspectiveDrag,
  rectToQuad,
  toAffine,
  type Homography,
} from "../src/lib/homography";

const BOX = { left: 0, top: 0, right: 100, bottom: 50 };
const close = (p: number[], q: number[]) => p.forEach((v, i) => expect(v).toBeCloseTo(q[i], 9));

test("a rectangle goes to the quad asked, its sides straight", () => {
  const quad: [number, number][] = [
    [20, 0],
    [80, 0],
    [100, 50],
    [0, 50],
  ];
  const m = rectToQuad(BOX, quad)!;
  corners(BOX).forEach(([x, y], k) => close(apply(m, x, y), quad[k]));
  // The middle of the bottom edge stays on it.
  expect(apply(m, 50, 50)[1]).toBeCloseTo(50, 9);
  expect(toAffine(m)).toBeNull();
  // Three corners on a line: none.
  expect(
    rectToQuad(BOX, [
      [0, 0],
      [50, 0],
      [100, 0],
      [0, 50],
    ]),
  ).toBeNull();
});

test("a parallelogram is an affine map, given as six numbers", () => {
  const m = rectToQuad(BOX, [
    [10, 0],
    [110, 0],
    [100, 50],
    [0, 50],
  ])!;
  const affine = toAffine([...m.slice(0, 6), 0, 0, 1] as Homography)!;
  close(affine, [1, 0, -0.2, 1, 10, 0]);
  expect(Math.abs(m[6]) + Math.abs(m[7])).toBeLessThan(1e-12);
  expect(toAffine(fromAffine([2, 0, 0, 3, 4, 5]))).toEqual([2, 0, 0, 3, 4, 5]);
});

test("composition and inverse undo each other", () => {
  const m = rectToQuad(BOX, [
    [20, 5],
    [80, 0],
    [100, 50],
    [5, 45],
  ])!;
  const back = andThen(m, invert(m)!);
  close(apply(back, 37, 12), [37, 12]);
  const moved = andThen(m, fromAffine([1, 0, 0, 1, 10, -4]));
  close(apply(moved, 0, 0), [30, 1]);
});

test("only convex quads turning one way are places for a perspective", () => {
  expect(isConvex(corners(BOX))).toBe(true);
  // Crossed (a bow tie) or with a dent: refused.
  const crossed: [number, number][] = [
    [0, 0],
    [100, 50],
    [100, 0],
    [0, 50],
  ];
  expect(isConvex(crossed)).toBe(false);
  const dented: [number, number][] = [
    [0, 0],
    [100, 0],
    [40, 20],
    [0, 50],
  ];
  expect(isConvex(dented)).toBe(false);
});

test("Perspective moves the paired corner the other way, on the axis dragged most", () => {
  const start = corners(BOX);
  // The top-right corner dragged right: the top-left goes left, a wider top.
  expect(perspectiveDrag(start, 1, 10, 2)).toEqual([
    [-10, 0],
    [110, 0],
    [100, 50],
    [0, 50],
  ]);
  // The bottom-left corner dragged down: the top-left goes up.
  expect(perspectiveDrag(start, 3, 1, 8)).toEqual([
    [0, -8],
    [100, 0],
    [100, 50],
    [0, 58],
  ]);
});

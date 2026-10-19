import { expect, test } from "vitest";
import {
  OFF_GRID,
  SHEAR_POINTS,
  STRAIGHT,
  clientPoint,
  grabbedPoint,
  gridPoint,
  offGrid,
  shearOffset,
  shearPoints,
  shearValues,
  validShearPoints,
  withPointAdded,
  withPointKey,
  withPointMoved,
  withPointRemoved,
  type ShearPoint,
} from "../src/lib/shearCurve";

const box = { left: 10, top: 20, width: 200, height: 100 };

test("a curve's points are its values, a row and an offset each", () => {
  expect(shearPoints([0, 5, 40, -20, 100, 0])).toEqual([
    [0, 5],
    [40, -20],
    [100, 0],
  ]);
  expect(shearValues(STRAIGHT)).toEqual([0, 0, 100, 0]);
});

test("the engine's curves: 2 to 16 points from the top row to the bottom one", () => {
  expect(validShearPoints(STRAIGHT)).toBe(true);
  const rows = (n: number): ShearPoint[] =>
    Array.from({ length: n }, (_, i) => [(i * 100) / (n - 1), 0]);
  expect(validShearPoints(rows(SHEAR_POINTS))).toBe(true);
  expect(validShearPoints(rows(SHEAR_POINTS + 1))).toBe(false);
  expect(validShearPoints([[0, 0]])).toBe(false);
  expect(
    validShearPoints([
      [5, 0],
      [100, 0],
    ]),
  ).toBe(false);
  expect(
    validShearPoints([
      [0, 0],
      [60, 0],
      [40, 0],
      [100, 0],
    ]),
  ).toBe(false);
  expect(
    validShearPoints([
      [0, 101],
      [100, 0],
    ]),
  ).toBe(false);
  expect(
    validShearPoints([
      [0, Number.NaN],
      [100, 0],
    ]),
  ).toBe(false);
});

test("the line goes smoothly through the points, as the engine's spline", () => {
  const slant: ShearPoint[] = [
    [0, -40],
    [100, 60],
  ];
  expect(shearOffset(slant, 50)).toBeCloseTo(10);
  expect(shearOffset(slant, 25)).toBeCloseTo(-15);
  const bend: ShearPoint[] = [
    [0, 0],
    [30, 50],
    [70, -20],
    [100, 0],
  ];
  for (const [row, offset] of bend) expect(shearOffset(bend, row)).toBeCloseTo(offset);
  expect(shearOffset(bend, 15)).toBeGreaterThan(0);
  expect(shearOffset(bend, -10)).toBeCloseTo(0);
  // Overshooting the grid, it stays within it.
  const steep: ShearPoint[] = [
    [0, 100],
    [10, -100],
    [20, 100],
    [100, 0],
  ];
  for (let row = 0; row <= 100; row++) {
    expect(Math.abs(shearOffset(steep, row))).toBeLessThanOrEqual(100);
  }
});

test("the grid: the top row to the bottom one, half the frame either way", () => {
  expect(gridPoint(box, 110, 20)).toEqual([0, 0]);
  expect(gridPoint(box, 10, 120)).toEqual([100, -100]);
  expect(gridPoint(box, 160, 70)).toEqual([50, 50]);
  expect(gridPoint(box, 500, -5)).toEqual([0, 100]);
  expect(gridPoint(box, 110.13, 53.33)).toEqual([33.3, 0.1]);
  expect(clientPoint(box, [50, 50])).toEqual([160, 70]);
  // A press near a point grabs it, the nearest; elsewhere none.
  const points: ShearPoint[] = [
    [0, 0],
    [50, 50],
    [100, 0],
  ];
  expect(grabbedPoint(points, box, 158, 72)).toBe(1);
  expect(grabbedPoint(points, box, 110, 21)).toBe(0);
  expect(grabbedPoint(points, box, 130, 70)).toBe(-1);
  expect(offGrid(box, 10 - OFF_GRID - 1, 50)).toBe(true);
  expect(offGrid(box, 10 - OFF_GRID + 1, 50)).toBe(false);
  expect(offGrid(box, 100, 120 + OFF_GRID + 1)).toBe(true);
});

test("points added between the others, up to 16, never on another's row", () => {
  const added = withPointAdded(STRAIGHT, [40, 25]);
  expect(added).toEqual({
    points: [
      [0, 0],
      [40, 25],
      [100, 0],
    ],
    index: 1,
  });
  expect(withPointAdded(added!.points, [40.5, 0])).toBeNull();
  expect(withPointAdded(STRAIGHT, [0, 30])).toBeNull();
  expect(withPointAdded(STRAIGHT, [100, 30])).toBeNull();
  const full: ShearPoint[] = Array.from({ length: SHEAR_POINTS }, (_, i) => [
    (i * 100) / (SHEAR_POINTS - 1),
    0,
  ]);
  expect(withPointAdded(full, [3, 0])).toBeNull();
});

test("points moved: the ends along their row, the others between their neighbours", () => {
  const points: ShearPoint[] = [
    [0, 0],
    [40, 10],
    [60, 0],
    [100, 0],
  ];
  expect(withPointMoved(points, 0, [30, -150])[0]).toEqual([0, -100]);
  expect(withPointMoved(points, 3, [20, 70])[3]).toEqual([100, 70]);
  expect(withPointMoved(points, 1, [80, 5])[1]).toEqual([59, 5]);
  expect(withPointMoved(points, 2, [10, 5])[2]).toEqual([41, 5]);
  expect(withPointRemoved(points, 1)).toEqual([
    [0, 0],
    [60, 0],
    [100, 0],
  ]);
  expect(withPointRemoved(points, 0)).toBe(points);
  expect(withPointRemoved(points, 3)).toBe(points);
  // The keys: arrows by 1 %, by 10 % with Shift, Delete.
  expect(withPointKey(points, 1, "ArrowRight", false)?.[1]).toEqual([40, 11]);
  expect(withPointKey(points, 1, "ArrowLeft", true)?.[1]).toEqual([40, 0]);
  expect(withPointKey(points, 1, "ArrowDown", false)?.[1]).toEqual([41, 10]);
  expect(withPointKey(points, 0, "ArrowDown", false)?.[0]).toEqual([0, 0]);
  expect(withPointKey(points, 2, "Delete", false)).toHaveLength(3);
  expect(withPointKey(points, 1, "a", false)).toBeNull();
});

import { describe, expect, test } from "vitest";
import {
  boxPoint,
  frameOfDrag,
  handlePoint,
  inFrame,
  resizeFrame,
  type Corners,
  type Frame,
} from "../src/lib/textFrame";

/** A box 100 by 50 at (10, 20), placed straight in the document. */
const frame: Frame = { position: [10, 20], size: [100, 50] };
const straight: Corners = [
  [10, 20],
  [110, 20],
  [110, 70],
  [10, 70],
];

describe("a drawn box", () => {
  test("runs from the drag's top left, whichever way it went", () => {
    expect(frameOfDrag([110, 70], [10, 20])).toEqual(frame);
    expect(frameOfDrag([10, 20], [110, 70])).toEqual(frame);
  });

  test("is never empty", () => {
    expect(frameOfDrag([10, 20], [10, 90]).size).toEqual([1, 70]);
  });
});

describe("points in a box", () => {
  test("are measured from its top left, along its edges", () => {
    expect(boxPoint(straight, frame.size, [60, 45])).toEqual([50, 25]);
    // Turned a quarter clockwise and twice as large: its top runs down the document.
    const turned: Corners = [
      [200, 0],
      [200, 200],
      [100, 200],
      [100, 0],
    ];
    const [x, y] = boxPoint(turned, frame.size, [150, 50])!;
    expect(x).toBeCloseTo(25);
    expect(y).toBeCloseTo(25);
    expect(inFrame(turned, frame.size, [150, 50])).toBe(true);
    expect(inFrame(turned, frame.size, [250, 50])).toBe(false);
  });

  test("of a flattened box are nowhere", () => {
    const flat: Corners = [
      [0, 0],
      [10, 0],
      [10, 0],
      [0, 0],
    ];
    expect(boxPoint(flat, [10, 10], [5, 0])).toBeNull();
  });
});

describe("handles", () => {
  test("stand at the corners and the middles of the edges, clockwise", () => {
    expect([0, 1, 2, 3, 4, 5, 6, 7].map((i) => handlePoint(straight, i))).toEqual([
      [10, 20],
      [60, 20],
      [110, 20],
      [110, 45],
      [110, 70],
      [60, 70],
      [10, 70],
      [10, 45],
    ]);
  });

  test("move the edges they hold", () => {
    // The right edge.
    expect(resizeFrame(frame, straight, 3, [150, 0])).toEqual({
      position: [10, 20],
      size: [140, 50],
    });
    // The top left corner: the box's position follows.
    expect(resizeFrame(frame, straight, 0, [20, 30])).toEqual({
      position: [20, 30],
      size: [90, 40],
    });
    // The bottom: only the height.
    expect(resizeFrame(frame, straight, 5, [500, 90])).toEqual({
      position: [10, 20],
      size: [100, 70],
    });
  });

  test("keep the box from turning inside out", () => {
    expect(resizeFrame(frame, straight, 3, [0, 0])).toEqual({ position: [10, 20], size: [1, 50] });
    expect(resizeFrame(frame, straight, 7, [500, 0])).toEqual({
      position: [109, 20],
      size: [1, 50],
    });
  });
});

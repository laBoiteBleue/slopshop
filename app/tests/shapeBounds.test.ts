import { describe, expect, test } from "vitest";
import { documentBounds, geometryBounds, withBounds } from "../src/lib/shapeBounds";
import type { ShapeGeometry } from "../src/lib/shapes";
import { changedShape, type Shape } from "../src/lib/shapes";

const ID: [number, number, number, number, number, number] = [1, 0, 0, 1, 0, 0];
const RECT: ShapeGeometry = { kind: "rectangle", rect: [10, 20, 110, 70], radii: [5, 5, 5, 5] };

describe("geometryBounds", () => {
  test("a rectangle, an ellipse, a line", () => {
    expect(geometryBounds(RECT)).toEqual({ left: 10, top: 20, right: 110, bottom: 70 });
    expect(geometryBounds({ kind: "ellipse", center: [50, 50], radii: [30, 10] })).toEqual({
      left: 20,
      top: 40,
      right: 80,
      bottom: 60,
    });
    expect(geometryBounds({ kind: "line", from: [5, 9], to: [1, 3] })).toEqual({
      left: 1,
      top: 3,
      right: 5,
      bottom: 9,
    });
  });

  test("a polygon's corners: a square standing on a corner", () => {
    const b = geometryBounds({
      kind: "polygon",
      center: [0, 0],
      radius: 10,
      sides: 4,
      star: null,
      rotation: 90,
    });
    expect(b.left).toBeCloseTo(-10);
    expect(b.top).toBeCloseTo(-10);
    expect(b.right).toBeCloseTo(10);
    expect(b.bottom).toBeCloseTo(10);
  });

  test("a path's curves, not its handles", () => {
    // A cubic from (0, 0) to (100, 0) with both handles at y = -40 peaks at y = -30.
    const b = geometryBounds({
      kind: "path",
      rule: "nonzero",
      subpaths: [{ start: [0, 0], segments: [[0, -40, 100, -40, 100, 0]], closed: false }],
    });
    expect(b.left).toBeCloseTo(0);
    expect(b.right).toBeCloseTo(100);
    expect(b.top).toBeCloseTo(-30);
    expect(b.bottom).toBeCloseTo(0);
  });
});

describe("documentBounds", () => {
  test("through the layer's translation and scale, flips included", () => {
    expect(documentBounds(RECT, [2, 0, 0, 1, 5, 0])).toEqual({
      x: 25,
      y: 20,
      width: 200,
      height: 50,
    });
    expect(documentBounds(RECT, [-1, 0, 0, 1, 0, 0])).toEqual({
      x: -110,
      y: 20,
      width: 100,
      height: 50,
    });
  });

  test("none when the layer rotates, skews or is in perspective", () => {
    expect(documentBounds(RECT, [0, 1, -1, 0, 0, 0])).toBeNull();
    expect(documentBounds(RECT, [1, 0, 0.5, 1, 0, 0])).toBeNull();
    expect(documentBounds(RECT, null)).toBeNull();
  });
});

describe("withBounds", () => {
  test("X and Y move a rectangle, W and H scale it from its top left, radii kept", () => {
    expect(withBounds(RECT, ID, "x", 0)).toEqual({ ...RECT, rect: [0, 20, 100, 70] });
    expect(withBounds(RECT, ID, "y", 0)).toEqual({ ...RECT, rect: [10, 0, 110, 50] });
    expect(withBounds(RECT, ID, "width", 50)).toEqual({ ...RECT, rect: [10, 20, 60, 70] });
    expect(withBounds(RECT, ID, "height", 100)).toEqual({ ...RECT, rect: [10, 20, 110, 120] });
  });

  test("values in the document, the layer's map undone", () => {
    // The layer doubles widths and moves 5 pixels right: 100 document pixels are 50 of its own.
    const g = withBounds(RECT, [2, 0, 0, 1, 5, 0], "width", 100);
    expect(g).toEqual({ ...RECT, rect: [10, 20, 60, 70] });
    expect(withBounds(RECT, [2, 0, 0, 1, 5, 0], "x", 5)).toEqual({
      ...RECT,
      rect: [0, 20, 100, 70],
    });
  });

  test("an ellipse's center and radii", () => {
    const g = withBounds({ kind: "ellipse", center: [50, 50], radii: [30, 10] }, ID, "width", 120);
    expect(g).toEqual({ kind: "ellipse", center: [80, 50], radii: [60, 10] });
  });

  test("a polygon stays regular: its other side follows", () => {
    const polygon: ShapeGeometry = {
      kind: "polygon",
      center: [0, 0],
      radius: 10,
      sides: 4,
      star: null,
      rotation: 90,
    };
    const g = withBounds(polygon, ID, "width", 40);
    expect(g).toMatchObject({ kind: "polygon", radius: 20, sides: 4 });
    const b = documentBounds(g as ShapeGeometry, ID);
    expect(b?.x).toBeCloseTo(-10);
    expect(b?.y).toBeCloseTo(-10);
    expect(b?.width).toBeCloseTo(40);
    expect(b?.height).toBeCloseTo(40);
  });

  test("a path's points and handles all follow", () => {
    const path: ShapeGeometry = {
      kind: "path",
      rule: "evenodd",
      subpaths: [
        {
          start: [0, 0],
          segments: [
            [10, 0],
            [10, 5, 5, 10, 0, 10],
          ],
          closed: true,
        },
      ],
    };
    expect(withBounds(path, ID, "width", 20)).toEqual({
      kind: "path",
      rule: "evenodd",
      subpaths: [
        {
          start: [0, 0],
          segments: [
            [20, 0],
            [20, 5, 10, 10, 0, 10],
          ],
          closed: true,
        },
      ],
    });
  });

  test("nothing for a size of zero or less, a flat side, a rotated layer", () => {
    expect(withBounds(RECT, ID, "width", 0)).toBeNull();
    expect(withBounds(RECT, ID, "height", -5)).toBeNull();
    const line: ShapeGeometry = { kind: "line", from: [0, 0], to: [10, 0] };
    expect(withBounds(line, ID, "height", 10)).toBeNull();
    expect(withBounds(line, ID, "width", 20)).toEqual({ ...line, to: [20, 0] });
    expect(withBounds(RECT, [0, 1, -1, 0, 0, 0], "x", 0)).toBeNull();
  });
});

describe("changedShape radii", () => {
  test("each corner its own radius, none past half the shorter side", () => {
    const shape: Shape = { geometry: RECT, fill: [0, 0, 0, 1], stroke: null };
    expect(changedShape(shape, { radii: [0, 10, 40, -3] }).geometry).toEqual({
      ...RECT,
      radii: [0, 10, 25, 0],
    });
  });
});

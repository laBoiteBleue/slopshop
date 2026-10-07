import { expect, test } from "vitest";
import {
  boxGeometry,
  changedShape,
  defaultShapeOptions,
  dragBox,
  hasExtent,
  hexOf,
  outlinePoints,
  shapeKindOf,
  shapeOf,
  svgPath,
} from "../src/lib/shapes";

const options = defaultShapeOptions("#ff0000");

test("a drag draws a box: from corner to corner, a square with Shift, from the center with Alt", () => {
  expect(dragBox([10, 10], [40, 30], false, false)).toEqual({
    left: 10,
    top: 10,
    right: 40,
    bottom: 30,
  });
  // Upward and leftward drags give the same box.
  expect(dragBox([40, 30], [10, 10], false, false)).toEqual({
    left: 10,
    top: 10,
    right: 40,
    bottom: 30,
  });
  expect(dragBox([10, 10], [40, 20], true, false)).toEqual({
    left: 10,
    top: 10,
    right: 40,
    bottom: 40,
  });
  expect(dragBox([50, 50], [60, 70], false, true)).toEqual({
    left: 40,
    top: 30,
    right: 60,
    bottom: 70,
  });
});

test("each tool draws its geometry in the box", () => {
  const box = { left: 0, top: 0, right: 100, bottom: 40 };
  // Corners round by the radius, never past half the shorter side.
  expect(boxGeometry("rectangle", box, { ...options, radius: 50 })).toEqual({
    kind: "rectangle",
    rect: [0, 0, 100, 40],
    radii: [20, 20, 20, 20],
  });
  expect(boxGeometry("ellipse", box, options)).toEqual({
    kind: "ellipse",
    center: [50, 20],
    radii: [50, 20],
  });
  // A polygon within the shorter side, its first corner up; a star with its indent.
  expect(boxGeometry("polygon", box, { ...options, sides: 6 })).toEqual({
    kind: "polygon",
    center: [50, 20],
    radius: 20,
    sides: 6,
    star: null,
    rotation: 90,
  });
  expect(
    boxGeometry("polygon", box, { ...options, sides: 500, star: true, starRatio: 0.4 }),
  ).toMatchObject({ sides: 100, star: 0.4 });
});

test("the options give the fill and stroke; a line draws with the stroke's color", () => {
  const rect = boxGeometry("rectangle", { left: 0, top: 0, right: 10, bottom: 10 }, options);
  expect(shapeOf(rect, options)).toEqual({ geometry: rect, fill: [1, 0, 0, 1], stroke: null });
  const outlined = shapeOf(rect, {
    ...options,
    filled: false,
    stroked: true,
    stroke: "#0000ff",
    strokeWidth: 4,
    strokeAlign: "outside",
  });
  expect(outlined).toEqual({
    geometry: rect,
    fill: null,
    stroke: {
      color: [0, 0, 1, 1],
      width: 4,
      align: "outside",
      cap: "butt",
      join: "miter",
      dashes: [],
    },
  });
  // Neither fill nor stroke: nothing to draw.
  expect(shapeOf(rect, { ...options, filled: false })).toBeNull();
  const line = {
    kind: "line" as const,
    from: [0, 0] as [number, number],
    to: [5, 5] as [number, number],
  };
  expect(shapeOf(line, { ...options, stroke: "#00ff00" })).toMatchObject({
    fill: null,
    stroke: { color: [0, 1, 0, 1], align: "center" },
  });
});

test("only a shape with an extent makes a layer", () => {
  expect(hasExtent({ kind: "line", from: [1, 1], to: [1, 1] })).toBe(false);
  expect(hasExtent({ kind: "ellipse", center: [0, 0], radii: [3, 0] })).toBe(false);
  expect(hasExtent({ kind: "rectangle", rect: [0, 0, 1, 1], radii: [0, 0, 0, 0] })).toBe(true);
});

test("the outline shown while dragging follows the shape", () => {
  expect(outlinePoints({ kind: "rectangle", rect: [0, 0, 4, 2], radii: [0, 0, 0, 0] })).toEqual([
    [0, 0],
    [4, 0],
    [4, 2],
    [0, 2],
  ]);
  // A star has twice its points' corners, the first one straight up.
  const star = outlinePoints({
    kind: "polygon",
    center: [0, 0],
    radius: 10,
    sides: 5,
    star: 0.5,
    rotation: 90,
  });
  expect(star).toHaveLength(10);
  expect(star[0][0]).toBeCloseTo(0);
  expect(star[0][1]).toBeCloseTo(-10);
  expect(Math.hypot(...star[1])).toBeCloseTo(5);
  // Rounded corners stay within the box.
  const round = outlinePoints({ kind: "rectangle", rect: [0, 0, 20, 10], radii: [5, 5, 5, 5] });
  expect(
    round.every(([x, y]) => x >= -1e-9 && x <= 20 + 1e-9 && y >= -1e-9 && y <= 10 + 1e-9),
  ).toBe(true);
  expect(
    svgPath(
      [
        [0, 0],
        [1, 2],
      ],
      (x, y) => [x * 2, y * 2],
      true,
    ),
  ).toBe("M0.00 0.00 L2.00 4.00");
  expect(
    svgPath(
      [
        [0, 0],
        [1, 2],
      ],
      (x, y) => [x, y],
      false,
    ),
  ).toBe("M0.00 0.00 L1.00 2.00 Z");
});

test("the shape tools are told apart from the others", () => {
  expect(shapeKindOf("shapePolygon")).toBe("polygon");
  expect(shapeKindOf("ellipse")).toBeNull();
});

test("the Properties panel's changes keep the rest of the shape", () => {
  const rect = {
    geometry: {
      kind: "rectangle" as const,
      rect: [0, 0, 10, 10] as [number, number, number, number],
      radii: [0, 0, 0, 0] as [number, number, number, number],
    },
    fill: [1, 0, 0, 1] as [number, number, number, number],
    stroke: null,
  };
  // A stroke's width or position without a stroke changes nothing.
  expect(changedShape(rect, { strokeWidth: 5, strokeAlign: "outside" })).toEqual(rect);
  const stroked = changedShape(rect, { stroke: "#00ff00" });
  expect(stroked.stroke).toMatchObject({ color: [0, 1, 0, 1], width: 3, align: "inside" });
  expect(changedShape(stroked, { strokeWidth: 0 }).stroke?.width).toBe(0.1);
  expect(changedShape(stroked, { fill: null }).fill).toBeNull();
  // A line keeps its stroke and has no fill; its stroke stays centered.
  const line = {
    geometry: {
      kind: "line" as const,
      from: [0, 0] as [number, number],
      to: [1, 1] as [number, number],
    },
    fill: null,
    stroke: {
      color: [0, 0, 0, 1] as [number, number, number, number],
      width: 1,
      align: "center" as const,
      cap: "butt" as const,
      join: "miter" as const,
      dashes: [],
    },
  };
  expect(changedShape(line, { stroke: null, fill: "#ffffff", strokeAlign: "inside" })).toEqual(
    line,
  );
  // A polygon's sides within bounds, a star's indent.
  const polygon = {
    geometry: {
      kind: "polygon" as const,
      center: [0, 0] as [number, number],
      radius: 5,
      sides: 5,
      star: null,
      rotation: 90,
    },
    fill: null,
    stroke: null,
  };
  expect(changedShape(polygon, { sides: 2, star: 0.3 }).geometry).toMatchObject({
    sides: 3,
    star: 0.3,
  });
  expect(hexOf([1, 0.5, 0, 1])).toBe("#ff8000");
});

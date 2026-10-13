import { describe, expect, test } from "vitest";
import {
  closed,
  corner,
  dragged,
  geometryOf,
  isDrawable,
  nextPoint,
  rubberBand,
  svgOfPath,
  withAnchor,
  withoutLast,
  type PenPath,
} from "../src/lib/pen";

const identity = (x: number, y: number): [number, number] => [x, y];

function path(...points: [number, number][]): PenPath {
  return points.reduce<PenPath | null>((p, point) => withAnchor(p, corner(point)), null)!;
}

describe("anchors", () => {
  test("a drag makes symmetric handles; Alt's frozen handle stays put", () => {
    expect(dragged([10, 10], [20, 15], false, null)).toEqual({
      point: [10, 10],
      in: [0, 5],
      out: [20, 15],
    });
    expect(dragged([10, 10], [20, 15], false, [4, 4])).toEqual({
      point: [10, 10],
      in: [4, 4],
      out: [20, 15],
    });
  });

  test("Shift puts the handle and the next anchor on 45° steps", () => {
    const a = dragged([0, 0], [10, 1], true, null);
    expect(a.out[0]).toBeCloseTo(Math.hypot(10, 1));
    expect(a.out[1]).toBeCloseTo(0);
    expect(a.in[0]).toBeCloseTo(-Math.hypot(10, 1));
    const p = path([0, 0]);
    const next = nextPoint(p, [10, 9], true);
    expect(next[0]).toBeCloseTo(next[1]);
    expect(nextPoint(p, [10, 9], false)).toEqual([10, 9]);
    // Nothing to constrain from before the first anchor.
    expect(nextPoint(null, [10, 9], true)).toEqual([10, 9]);
  });

  test("Backspace removes the last anchor, then the path", () => {
    const p = path([0, 0], [10, 0]);
    expect(withoutLast(p)).toEqual(path([0, 0]));
    expect(withoutLast(path([0, 0]))).toBeNull();
  });

  test("a path is worth a layer from two anchors", () => {
    expect(isDrawable(null)).toBe(false);
    expect(isDrawable(path([0, 0]))).toBe(false);
    expect(isDrawable(path([0, 0], [1, 1]))).toBe(true);
  });
});

describe("geometry", () => {
  test("corners make lines; an open path stays open", () => {
    expect(geometryOf(path([0, 0], [10, 0], [10, 10]))).toEqual({
      kind: "path",
      rule: "nonzero",
      subpaths: [
        {
          start: [0, 0],
          segments: [
            [10, 0],
            [10, 10],
          ],
          closed: false,
        },
      ],
    });
  });

  test("handles make cubics, and closing adds the segment back to the first anchor", () => {
    const smooth = dragged([10, 0], [15, 0], false, null);
    const p = closed(withAnchor(path([0, 0]), smooth));
    expect(geometryOf(p).subpaths[0]).toEqual({
      start: [0, 0],
      segments: [
        [0, 0, 5, 0, 10, 0],
        [15, 0, 0, 0, 0, 0],
      ],
      closed: true,
    });
    // A closing drag reshapes the first anchor's handles.
    const reshaped = closed(path([0, 0], [10, 0]), dragged([0, 0], [0, 5], false, null));
    expect(geometryOf(reshaped).subpaths[0].segments).toEqual([
      [0, 5, 10, 0, 10, 0],
      [10, 0, 0, -5, 0, 0],
    ]);
  });

  test("an SVG path of lines and cubics, mapped to the screen", () => {
    const smooth = dragged([10, 0], [15, 0], false, null);
    const g = geometryOf(closed(withAnchor(path([0, 0]), smooth)));
    expect(svgOfPath(g, (x, y) => [2 * x, 2 * y + 1])).toBe(
      "M0.00 1.00 C0.00 1.00 10.00 1.00 20.00 1.00 C30.00 1.00 0.00 1.00 0.00 1.00 Z",
    );
    expect(svgOfPath(geometryOf(path([0, 0], [3, 4])), identity)).toBe("M0.00 0.00 L3.00 4.00");
  });

  test("the rubber band bends by the last anchor's outgoing handle", () => {
    expect(rubberBand(null, [5, 5])).toBeNull();
    expect(rubberBand(path([0, 0]), [5, 5])?.subpaths[0].segments).toEqual([[5, 5]]);
    const smooth = withAnchor(null, dragged([0, 0], [4, 0], false, null));
    expect(rubberBand(smooth, [10, 10])?.subpaths[0]).toEqual({
      start: [0, 0],
      segments: [[4, 0, 10, 10, 10, 10]],
      closed: false,
    });
  });
});

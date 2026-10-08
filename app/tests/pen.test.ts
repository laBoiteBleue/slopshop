import { expect, test } from "vitest";
import type { LayerView } from "../src/lib/engine";
import {
  anchorsOf,
  contentToDocument,
  drawable,
  geometryOf,
  hitAt,
  moved,
  pathsOf,
  press,
  pull,
  svgOf,
  withoutLast,
  type PenPath,
} from "../src/lib/pen";

const corner = (x: number, y: number) => ({
  point: [x, y] as [number, number],
  before: null,
  after: null,
});

test("clicks place corners, a drag a smooth anchor, a click on the first anchor closes", () => {
  let path = press(null, [10, 10], 5);
  path = press(path, [100, 10], 5);
  // A drag after the press: the handle after it there, the one before it opposite.
  path = pull(path, [120, 30], false);
  expect(path.anchors[1]).toEqual({ point: [100, 10], before: [80, -10], after: [120, 30] });
  path = press(path, [100, 90], 5);
  // Within reach of the first anchor: closed, no new anchor.
  path = press(path, [12, 13], 5);
  expect(path.closed).toBe(true);
  expect(path.anchors).toHaveLength(3);
  // A drag after closing shapes the first anchor.
  path = pull(path, [0, 20], false);
  expect(path.anchors[0]).toEqual({ point: [10, 10], before: [20, 0], after: [0, 20] });
  expect(geometryOf([path])).toEqual({
    kind: "path",
    evenOdd: false,
    subpaths: [
      {
        start: [10, 10],
        closed: true,
        segments: [
          { kind: "cubic", c1: [0, 20], c2: [80, -10], to: [100, 10] },
          { kind: "cubic", c1: [120, 30], c2: [100, 90], to: [100, 90] },
          { kind: "cubic", c1: [100, 90], c2: [20, 0], to: [10, 10] },
        ],
      },
    ],
  });
});

test("Alt while pulling breaks the handles; a lone anchor closes nothing", () => {
  const path = pull(press(press(null, [0, 0], 5), [50, 0], 5), [60, 10], false);
  const broken = pull(path, [70, 40], true);
  expect(broken.anchors[1].before).toEqual(path.anchors[1].before);
  expect(broken.anchors[1].after).toEqual([70, 40]);
  // One anchor: a press on it adds another rather than closing.
  expect(press(press(null, [0, 0], 5), [1, 1], 5).anchors).toHaveLength(2);
});

test("Backspace takes the last anchor; a path of one point draws nothing", () => {
  const path = press(press(null, [0, 0], 5), [40, 0], 5);
  expect(withoutLast(path)?.anchors).toHaveLength(1);
  expect(withoutLast(withoutLast(path)!)).toBeNull();
  expect(drawable(path)).toBe(true);
  expect(drawable({ anchors: [corner(3, 3)], closed: false })).toBe(false);
  expect(drawable({ anchors: [corner(3, 3), corner(3, 3)], closed: false })).toBe(false);
  // Straight segments between corners.
  expect(geometryOf([path]).subpaths[0].segments).toEqual([{ kind: "line", to: [40, 0] }]);
  expect(geometryOf([{ anchors: [corner(1, 1)], closed: false }]).subpaths).toEqual([]);
});

test("a path geometry gives its anchors back, a closed one its first anchor's handle", () => {
  const paths: PenPath[] = [
    {
      anchors: [
        { point: [0, 0], before: [-5, 5], after: [5, -5] },
        corner(50, 0),
        { point: [50, 50], before: [60, 40], after: null },
      ],
      closed: true,
    },
    { anchors: [corner(100, 100), corner(120, 100)], closed: false },
  ];
  expect(pathsOf(geometryOf(paths))).toEqual(paths);
});

test("live shapes become anchors: corners, rounded corners and an ellipse's quarters", () => {
  const [rect] = anchorsOf({ kind: "rectangle", rect: [0, 0, 10, 20], radii: [0, 0, 0, 0] });
  expect(rect.anchors.map((a) => a.point)).toEqual([
    [0, 0],
    [10, 0],
    [10, 20],
    [0, 20],
  ]);
  const [rounded] = anchorsOf({
    kind: "rectangle",
    rect: [0, 0, 100, 50],
    radii: [10, 10, 10, 10],
  });
  expect(rounded.anchors).toHaveLength(8);
  expect(rounded.closed).toBe(true);
  const [ellipse] = anchorsOf({ kind: "ellipse", center: [50, 50], radii: [40, 20] });
  expect(ellipse.anchors.map((a) => a.point)).toEqual([
    [90, 50],
    [50, 70],
    [10, 50],
    [50, 30],
  ]);
  // Its quarters are cubics whose midpoints lie on the ellipse.
  const quarter = geometryOf([ellipse]).subpaths[0].segments[0];
  expect(quarter.kind).toBe("cubic");
  if (quarter.kind === "cubic") {
    const x = (90 + 3 * quarter.c1[0] + 3 * quarter.c2[0] + quarter.to[0]) / 8;
    const y = (50 + 3 * quarter.c1[1] + 3 * quarter.c2[1] + quarter.to[1]) / 8;
    expect(((x - 50) / 40) ** 2 + ((y - 50) / 20) ** 2).toBeCloseTo(1, 3);
  }
  const [star] = anchorsOf({
    kind: "polygon",
    center: [0, 0],
    radius: 10,
    sides: 5,
    star: 0.5,
    rotation: 90,
  });
  expect(star.anchors).toHaveLength(10);
  expect(star.anchors[0].point[1]).toBeCloseTo(-10);
  const [line] = anchorsOf({ kind: "line", from: [1, 2], to: [3, 4] });
  expect(line).toEqual({ anchors: [corner(1, 2), corner(3, 4)], closed: false });
});

test("Direct Selection takes handles before anchors, and moves them as Photoshop does", () => {
  const paths: PenPath[] = [
    {
      anchors: [corner(0, 0), { point: [50, 0], before: [40, 0], after: [70, 0] }],
      closed: false,
    },
  ];
  expect(hitAt(paths, [41, 1], 3)).toEqual({ path: 0, anchor: 1, part: "before" });
  expect(hitAt(paths, [50, 2], 3)).toEqual({ path: 0, anchor: 1, part: "point" });
  expect(hitAt(paths, [25, 25], 3)).toBeNull();
  // An anchor carries its handles.
  const point = moved(paths, { path: 0, anchor: 1, part: "point" }, [50, 10], false);
  expect(point[0].anchors[1]).toEqual({ point: [50, 10], before: [40, 10], after: [70, 10] });
  // A handle turns the other, which keeps its length.
  const handle = moved(paths, { path: 0, anchor: 1, part: "before" }, [50, -10], false);
  expect(handle[0].anchors[1].before).toEqual([50, -10]);
  expect(handle[0].anchors[1].after?.[0]).toBeCloseTo(50);
  expect(handle[0].anchors[1].after?.[1]).toBeCloseTo(20);
  // Alt: the other stays.
  const broken = moved(paths, { path: 0, anchor: 1, part: "before" }, [50, -10], true);
  expect(broken[0].anchors[1].after).toEqual([70, 0]);
  expect(paths[0].anchors[1].before).toEqual([40, 0]);
});

test("the outline drawn on screen: lines, curves and a close, mapped", () => {
  const path: PenPath = {
    anchors: [corner(0, 0), { point: [10, 0], before: [5, 5], after: null }],
    closed: true,
  };
  expect(svgOf([path], (x, y) => [2 * x, 2 * y + 1])).toBe(
    "M0.00 1.00 C0.00 1.00 10.00 11.00 20.00 1.00 L0.00 1.00 Z",
  );
});

test("a layer's content is placed by its transform and its groups'", () => {
  const layer = (id: number, transform: number[], children: LayerView[] = []) =>
    ({ id, transform, children, perspective: null }) as unknown as LayerView;
  const tree = [layer(1, [1, 0, 0, 1, 100, 0], [layer(2, [2, 0, 0, 2, 0, 10])])];
  // The layer's, then its group's.
  expect(contentToDocument(tree, 2)).toEqual([2, 0, 0, 2, 100, 10]);
  expect(contentToDocument(tree, 9)).toBeNull();
  const tilted = [{ ...tree[0], perspective: [1, 0, 0, 0, 1, 0, 0.001, 0, 1] } as LayerView];
  expect(contentToDocument(tilted, 2)).toBeNull();
});

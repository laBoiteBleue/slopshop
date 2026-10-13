import { describe, expect, test } from "vitest";
import type { LayerView } from "../src/lib/engine";
import {
  cornered,
  editablePath,
  geometryOfPaths,
  hitTest,
  isEnd,
  isSmooth,
  layerToDocument,
  movedAnchor,
  movedHandle,
  movedPath,
  pathsOf,
  removed,
  reversed,
  split,
} from "../src/lib/pathEdit";
import { corner, type PathGeometry, type PenPath, type Point } from "../src/lib/pen";

const identity = (p: Point): Point => p;

/** A closed curve as the Pen draws it: its last segment back to its start. */
const drop: PathGeometry = {
  kind: "path",
  rule: "nonzero",
  subpaths: [
    {
      start: [0, 0],
      segments: [
        [100, 0],
        [100, 0, 100, 100, 50, 100],
        [0, 100, 0, 0, 0, 0],
      ],
      closed: true,
    },
  ],
};

const line: PenPath = { anchors: [corner([0, 0]), corner([100, 0])], closed: false };

describe("paths as anchors", () => {
  test("a closed path's last segment gives its start its incoming handle", () => {
    const [p] = pathsOf(drop);
    expect(p.closed).toBe(true);
    expect(p.anchors).toEqual([
      { point: [0, 0], in: [0, 0], out: [0, 0] },
      { point: [100, 0], in: [100, 0], out: [100, 0] },
      { point: [50, 100], in: [100, 100], out: [0, 100] },
    ]);
    // Back to the same geometry.
    expect(geometryOfPaths([p], "nonzero")).toEqual(drop);
  });

  test("a closed path without its closing segment gets a straight one", () => {
    const square: PathGeometry = {
      kind: "path",
      rule: "evenodd",
      subpaths: [{ start: [0, 0], segments: [[10, 0], [10, 10]], closed: true }],
    };
    const [p] = pathsOf(square);
    expect(p.anchors).toHaveLength(3);
    expect(geometryOfPaths([p], "evenodd").subpaths[0].segments).toEqual([
      [10, 0],
      [10, 10],
      [0, 0],
    ]);
  });
});

describe("edits", () => {
  test("an anchor moves with its handles; the whole path moves", () => {
    const [p] = pathsOf(drop);
    expect(movedAnchor(p, 2, 5, -5).anchors[2]).toEqual({
      point: [55, 95],
      in: [105, 95],
      out: [5, 95],
    });
    expect(movedPath(p, 1, 2).anchors[0].point).toEqual([1, 2]);
  });

  test("a smooth anchor's handles turn together, unless Alt frees one", () => {
    const [p] = pathsOf(drop);
    expect(isSmooth(p.anchors[2])).toBe(true);
    expect(isSmooth(p.anchors[0])).toBe(false);
    const turned = movedHandle(p, 2, "out", [50, 50], false).anchors[2];
    expect(turned.out).toEqual([50, 50]);
    // The other handle opposite, its length (50) kept.
    expect(turned.in[0]).toBeCloseTo(50);
    expect(turned.in[1]).toBeCloseTo(150);
    const freed = movedHandle(p, 2, "out", [50, 50], true).anchors[2];
    expect(freed.in).toEqual([100, 100]);
  });

  test("Alt+click makes a corner", () => {
    const [p] = pathsOf(drop);
    expect(cornered(p, 2).anchors[2]).toEqual(corner([50, 100]));
  });

  test("an anchor added on a curve leaves it unchanged; on a line, a corner", () => {
    const [p] = pathsOf(drop);
    const added = split(p, 1, 0.5);
    expect(added.anchors).toHaveLength(4);
    // De Casteljau at t = 0.5 of (100,0) (100,0) (100,100) (50,100).
    expect(added.anchors[2].point).toEqual([93.75, 50]);
    expect(added.anchors[1].out).toEqual([100, 0]);
    expect(added.anchors[3].in).toEqual([75, 100]);
    expect(split(line, 0, 0.25).anchors[1]).toEqual(corner([25, 0]));
    // On the closing segment: added last.
    expect(split(p, 2, 0.5).anchors).toHaveLength(4);
  });

  test("deleting an anchor keeps two at least", () => {
    const [p] = pathsOf(drop);
    expect(removed(p, 1)?.anchors.map((a) => a.point)).toEqual([
      [0, 0],
      [50, 100],
    ]);
    expect(removed(line, 0)).toBeNull();
  });

  test("an open path's ends continue it, drawn from either", () => {
    expect(isEnd(line, 0)).toBe(true);
    expect(isEnd(line, 1)).toBe(true);
    const [p] = pathsOf(drop);
    expect(isEnd(p, 0)).toBe(false);
    const curve: PenPath = {
      anchors: [
        { point: [0, 0], in: [0, 0], out: [10, 0] },
        { point: [20, 0], in: [20, 10], out: [20, 0] },
      ],
      closed: false,
    };
    expect(reversed(curve).anchors).toEqual([
      { point: [20, 0], in: [20, 0], out: [20, 10] },
      { point: [0, 0], in: [10, 0], out: [0, 0] },
    ]);
  });
});

describe("hits", () => {
  test("anchors before handles before segments", () => {
    const [p] = pathsOf(drop);
    expect(hitTest([p], [101, 1], 6, identity, true)).toEqual({
      kind: "anchor",
      path: 0,
      index: 1,
    });
    expect(hitTest([p], [99, 99], 6, identity, true)).toEqual({
      kind: "handle",
      path: 0,
      index: 2,
      which: "in",
    });
    // Handles only when asked (Ctrl or Alt).
    expect(hitTest([p], [99, 99], 6, identity, false)).toBeNull();
    const hit = hitTest([p], [50, 2], 6, identity, false);
    expect(hit).toMatchObject({ kind: "segment", path: 0, index: 0 });
    expect(hit?.kind === "segment" && hit.t).toBeCloseTo(0.5, 1);
    expect(hitTest([p], [40, 50], 6, identity, false)).toBeNull();
  });
});

describe("layers", () => {
  const base = {
    name: "",
    visible: true,
    opacity: 1,
    blendMode: "normal",
    contentKey: 0,
    hasAlpha: false,
    mask: null,
    swatch: [0, 0, 0, 1],
    adjustment: null,
    children: [],
    passThrough: false,
    clipped: false,
    painted: false,
    entries: [],
  };
  const vector = (id: number, extra: Partial<LayerView>): LayerView =>
    ({
      ...base,
      id,
      kind: "vector",
      transform: [1, 0, 0, 1, 0, 0],
      shape: { geometry: drop, fill: [0, 0, 0, 1], stroke: null },
      ...extra,
    }) as LayerView;

  test("a layer's map to the document goes through its groups; perspective has none", () => {
    const inner = vector(2, { transform: [2, 0, 0, 2, 0, 0] });
    const group = {
      ...base,
      id: 1,
      kind: "group",
      transform: [1, 0, 0, 1, 10, 20],
      children: [inner],
    } as unknown as LayerView;
    expect(layerToDocument([group], 2)).toEqual([2, 0, 0, 2, 10, 20]);
    const tilted = vector(3, { perspective: [1, 0, 0, 1, 0, 0, 0.001, 0, 1] });
    expect(layerToDocument([tilted], 3)).toBeNull();
  });

  test("a path edits as it is, a live shape through its live path", () => {
    const path = vector(1, {});
    expect(editablePath([path], path)).toMatchObject({ layerId: 1, live: false, geometry: drop });
    const rect = vector(2, {
      shape: {
        geometry: { kind: "rectangle", rect: [0, 0, 10, 10], radii: [0, 0, 0, 0] },
        fill: null,
        stroke: null,
      },
      livePath: drop,
    });
    expect(editablePath([rect], rect)).toMatchObject({ layerId: 2, live: true, geometry: drop });
    expect(editablePath([rect], null)).toBeNull();
  });
});

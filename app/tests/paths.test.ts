import { describe, expect, test } from "vitest";
import type { DocumentView, LayerView, PathView } from "../src/lib/engine";
import {
  chosenPath,
  loadMode,
  nextPathName,
  pathEntries,
  penDestination,
  penEditable,
  sameRef,
  withSubpaths,
} from "../src/lib/paths";
import type { PathGeometry } from "../src/lib/pen";

const triangle: PathGeometry = {
  kind: "path",
  rule: "nonzero",
  subpaths: [
    {
      start: [0, 0],
      segments: [
        [10, 0],
        [0, 10],
      ],
      closed: true,
    },
  ],
};

const path = (id: number, name: string, work = false): PathView => ({
  id,
  name,
  work,
  geometry: triangle,
});

function layer(id: number, name: string, changes: Partial<LayerView> = {}): LayerView {
  return {
    id,
    name,
    kind: "raster",
    visible: true,
    opacity: 1,
    blendMode: "normal",
    transform: [1, 0, 0, 1, 0, 0],
    children: [],
    ...changes,
  } as LayerView;
}

const shapeLayer = (id: number) =>
  layer(id, "Shape 1", {
    kind: "vector",
    transform: [1, 0, 0, 1, 5, 0],
    shape: { geometry: triangle, fill: [0, 0, 0, 1], stroke: null },
  });

const masked = (id: number) =>
  layer(id, "Cat", {
    vectorMask: {
      enabled: true,
      inverted: false,
      feather: 0,
      geometry: { kind: "ellipse", center: [5, 5], radii: [5, 5] },
      livePath: triangle,
    },
  });

function doc(paths: PathView[], layers: LayerView[] = []): DocumentView {
  return { id: 1, width: 100, height: 80, layers, paths } as DocumentView;
}

describe("the Paths panel's rows", () => {
  test("the document's paths in order, then the active layer's shape path and vector mask", () => {
    const shape = shapeLayer(2);
    const both = { ...shape, vectorMask: masked(2).vectorMask };
    const d = doc([path(3, "Outline"), path(4, "", true)], [both]);
    const entries = pathEntries(d, both);
    expect(entries.map((e) => [e.kind, e.name])).toEqual([
      ["path", "Outline"],
      ["work", ""],
      ["shape", "Shape 1"],
      ["vectorMask", "Shape 1"],
    ]);
    expect(entries.map((e) => e.ref)).toEqual([
      { kind: "path", id: 3 },
      { kind: "path", id: 4 },
      { kind: "shape", layer: 2 },
      { kind: "vectorMask", layer: 2 },
    ]);
    // Outlines in the document: the layer's moved by its transform (5 right).
    expect(entries[0].outline).toBe("M0.00 0.00 L10.00 0.00 L0.00 10.00 Z");
    expect(entries[2].outline).toBe("M5.00 0.00 L15.00 0.00 L5.00 10.00 Z");
    // A live mask shows as its path.
    expect(entries[3].outline).toBe("M5.00 0.00 L15.00 0.00 L5.00 10.00 Z");
  });

  test("no active layer, a pixel layer or a text layer: only the document's paths", () => {
    const d = doc([path(1, "Outline")], [layer(1, "Cat")]);
    expect(pathEntries(d, null)).toHaveLength(1);
    expect(pathEntries(d, layer(1, "Cat"))).toHaveLength(1);
    const text = { ...shapeLayer(2), text: {} } as LayerView;
    expect(pathEntries(doc([]), text)).toEqual([]);
    expect(pathEntries({ ...d, paths: undefined }, null)).toEqual([]);
  });

  test("a chosen path is kept while it is listed, in its document", () => {
    const shape = shapeLayer(2);
    const d = doc([path(3, "Outline")], [shape]);
    const choice = { documentId: 1, ref: { kind: "shape" as const, layer: 2 } };
    expect(chosenPath(choice, d, shape)).toEqual({ kind: "shape", layer: 2 });
    // Another layer active: its shape path is no longer listed.
    expect(chosenPath(choice, d, layer(5, "Cat"))).toBeNull();
    // Another document.
    expect(chosenPath(choice, { ...d, id: 2 }, shape)).toBeNull();
    const deleted = { documentId: 1, ref: { kind: "path" as const, id: 9 } };
    expect(chosenPath(deleted, d, shape)).toBeNull();
    expect(chosenPath(null, d, shape)).toBeNull();
    expect(sameRef({ kind: "path", id: 3 }, { kind: "path", id: 3 })).toBe(true);
    expect(sameRef({ kind: "path", id: 3 }, { kind: "shape", layer: 3 })).toBe(false);
  });
});

describe("what the Pen edits and draws into", () => {
  const shape = shapeLayer(2);
  const d = doc([path(3, "Outline"), path(4, "", true)], [shape, masked(6)]);

  test("a chosen document path, in document pixels", () => {
    const editable = penEditable(d, shape, { kind: "path", id: 3 }, false);
    expect(editable).toMatchObject({ pathId: 3, live: false, matrix: [1, 0, 0, 1, 0, 0] });
    expect(editable?.geometry).toEqual(triangle);
    expect(penDestination(editable, "shape")).toEqual({ kind: "path", id: 3 });
    expect(penEditable(d, shape, { kind: "path", id: 99 }, false)).toBeNull();
  });

  test("a chosen vector mask, the targeted one, else the active vector layer", () => {
    const cat = masked(6);
    const mask = penEditable(d, cat, { kind: "vectorMask", layer: 6 }, false);
    expect(mask).toMatchObject({ layerId: 6, vectorMask: true, live: true });
    expect(penDestination(mask, "path")).toEqual({ kind: "vectorMask" });
    expect(penEditable(d, cat, null, true)).toMatchObject({ vectorMask: true });
    const own = penEditable(d, shape, null, false);
    expect(own).toMatchObject({ layerId: 2 });
    expect(own?.vectorMask).toBeUndefined();
    // Nothing chosen: a new shape layer in Shape mode, a new Work Path in Path mode.
    expect(penDestination(own, "shape")).toEqual({ kind: "shapeLayer" });
    expect(penDestination(own, "path")).toEqual({ kind: "workPath" });
    expect(penDestination(null, "path")).toEqual({ kind: "workPath" });
  });

  test("a path drawn into a document path is added to its subpaths", () => {
    const drawn: PathGeometry = { ...triangle, rule: "evenodd" };
    const both = withSubpaths(triangle, drawn);
    expect(both.rule).toBe("nonzero");
    expect(both.subpaths).toHaveLength(2);
  });
});

test("new paths are named Path 1, Path 2… after the saved ones", () => {
  const format = (n: number) => `Path ${n}`;
  expect(nextPathName(doc([]), format)).toBe("Path 1");
  expect(nextPathName(doc([path(1, "Path 1"), path(2, "Path 3")]), format)).toBe("Path 2");
  // The Work Path's name does not count.
  expect(nextPathName(doc([path(1, "Path 1", true)]), format)).toBe("Path 1");
});

test("Ctrl+click's keys combine as the selection tools'", () => {
  expect(loadMode({ shiftKey: false, altKey: false })).toBe("replace");
  expect(loadMode({ shiftKey: true, altKey: false })).toBe("add");
  expect(loadMode({ shiftKey: false, altKey: true })).toBe("subtract");
  expect(loadMode({ shiftKey: true, altKey: true })).toBe("intersect");
});

import { describe, expect, test } from "vitest";
import type { LayerView, VectorMaskView } from "../src/lib/engine";
import type { PathGeometry } from "../src/lib/pen";
import {
  currentPathSource,
  editableVectorMask,
  maskOutline,
  referenceVectorMask,
  vectorMaskAddition,
  vectorMaskEnabledToggle,
  vectorMaskRemoval,
  withPathAdded,
} from "../src/lib/vectorMask";

const square: PathGeometry = {
  kind: "path",
  rule: "evenodd",
  subpaths: [
    {
      start: [0, 0],
      segments: [
        [10, 0],
        [10, 10],
        [0, 10],
      ],
      closed: true,
    },
  ],
};

const mask = (changes: Partial<VectorMaskView> = {}): VectorMaskView => ({
  enabled: true,
  inverted: false,
  feather: 0,
  geometry: square,
  livePath: null,
  ...changes,
});

const layer = (id: number, changes: Partial<LayerView> = {}): LayerView =>
  ({
    id,
    name: `layer ${id}`,
    visible: true,
    opacity: 1,
    blendMode: "normal",
    kind: "raster",
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
    transform: [1, 0, 0, 1, 0, 0],
    ...changes,
  }) as LayerView;

const shape = (id: number) =>
  layer(id, { kind: "vector", shape: { geometry: square, fill: [0, 0, 0, 1], stroke: null } });

describe("Layer > Vector Mask", () => {
  test("Reveal All and Hide All go on the selected layers without a vector mask", () => {
    const a = layer(1);
    const b = layer(2, { vectorMask: mask() });
    expect(vectorMaskAddition([a, b], b, "revealAll")).toEqual({
      kind: "addVectorMask",
      ids: [1],
      mask: "revealAll",
    });
    expect(vectorMaskAddition([b], b, "hideAll")).toBeNull();
  });

  test("Current Path masks the active layer with the other selected vector layer's path", () => {
    const target = layer(1);
    const path = shape(2);
    expect(currentPathSource([target, path], target)).toBe(path);
    expect(vectorMaskAddition([target, path], target, "currentPath")).toEqual({
      kind: "addVectorMask",
      ids: [1],
      mask: "currentPath",
      from: 2,
    });
    // The vector layer active: the other one is no path.
    expect(vectorMaskAddition([target, path], path, "currentPath")).toBeNull();
    // One layer, three, or a target with a vector mask already: nothing.
    expect(vectorMaskAddition([target], target, "currentPath")).toBeNull();
    expect(vectorMaskAddition([target, path, shape(3)], target, "currentPath")).toBeNull();
    const masked = layer(4, { vectorMask: mask() });
    expect(vectorMaskAddition([masked, path], masked, "currentPath")).toBeNull();
  });

  test("Disable, Enable and Delete follow the reference vector mask", () => {
    const on = layer(1, { vectorMask: mask() });
    const off = layer(2, { vectorMask: mask({ enabled: false }) });
    const none = layer(3);
    expect(referenceVectorMask([none, off], none)).toEqual(off.vectorMask);
    // The active one is off: both on.
    expect(vectorMaskEnabledToggle([on, off], off)).toEqual({
      kind: "setVectorMaskEnabled",
      id: 2,
      enabled: true,
    });
    expect(vectorMaskEnabledToggle([on, off], on)).toEqual({
      kind: "setVectorMaskEnabled",
      id: 1,
      enabled: false,
    });
    expect(vectorMaskEnabledToggle([none], none)).toBeNull();
    expect(vectorMaskRemoval([on, none, off])).toEqual({
      kind: "batch",
      edits: [
        { kind: "removeVectorMask", id: 1 },
        { kind: "removeVectorMask", id: 2 },
      ],
    });
    expect(vectorMaskRemoval([none])).toBeNull();
  });
});

describe("the Pen in a vector mask", () => {
  test("it edits the mask's path, a live shape's through its live path", () => {
    const moved = layer(1, { transform: [1, 0, 0, 1, 5, 0], vectorMask: mask() });
    expect(editableVectorMask([moved], moved)).toEqual({
      layerId: 1,
      geometry: square,
      live: false,
      matrix: [1, 0, 0, 1, 5, 0],
      vectorMask: true,
    });
    const live = layer(2, {
      vectorMask: mask({
        geometry: { kind: "ellipse", center: [5, 5], radii: [5, 5] },
        livePath: square,
      }),
    });
    expect(editableVectorMask([live], live)).toMatchObject({ live: true, geometry: square });
    expect(editableVectorMask([moved], layer(3))).toBeNull();
  });

  test("a path drawn is added to the mask's, in the layer's space", () => {
    const moved = layer(1, { transform: [2, 0, 0, 2, 100, 0], vectorMask: mask() });
    const target = editableVectorMask([moved], moved);
    const drawn: PathGeometry = {
      kind: "path",
      rule: "nonzero",
      subpaths: [{ start: [100, 0], segments: [[120, 20]], closed: false }],
    };
    expect(withPathAdded(target!, drawn)).toEqual({
      kind: "path",
      rule: "evenodd",
      subpaths: [...square.subpaths, { start: [0, 0], segments: [[10, 10]], closed: false }],
    });
    // Not placed by an affine map (perspective): nothing.
    expect(withPathAdded({ ...target!, matrix: null }, drawn)).toBeNull();
  });

  test("the thumbnail draws the outline where the layer is", () => {
    expect(maskOutline(mask(), [1, 0, 0, 1, 5, 0])).toBe(
      "M5.00 0.00 L15.00 0.00 L15.00 10.00 L5.00 10.00 Z",
    );
    expect(
      maskOutline(mask({ geometry: { kind: "ellipse", center: [0, 0], radii: [1, 1] } }), null),
    ).toBe("");
  });
});

import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import { ALIGNS, DISTRIBUTES, canDistribute } from "../src/lib/align";
import { layerTree } from "../src/lib/layerTree";

function layer(id: number, changes: Partial<LayerView> = {}): LayerView {
  return {
    id,
    name: `Layer ${id}`,
    visible: true,
    opacity: 1,
    kind: "raster",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: 0,
    hasAlpha: true,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
    ...changes,
  };
}

test("distributing needs three layers, a group counting as one", () => {
  const [a, b, c] = [layer(1), layer(2), layer(3)];
  const group = layer(4, { kind: "group", children: [layer(5), layer(6)] });
  const tree = layerTree([a, b, c, group]);
  assert.equal(canDistribute(tree, [a, b]), false);
  assert.equal(canDistribute(tree, [a, b, c]), true);
  // The group and its layers: one item, with `a`, two.
  assert.equal(canDistribute(tree, [a, group, ...group.children]), false);
  assert.equal(canDistribute(tree, group.children.concat(a)), true);
});

test("Photoshop's six alignments and the four distributions kept", () => {
  assert.deepEqual(
    ALIGNS.map((a) => a.id),
    ["left", "horizontalCenters", "right", "top", "verticalCenters", "bottom"],
  );
  assert.deepEqual(
    DISTRIBUTES.map((d) => d.id),
    ["horizontalCenters", "verticalCenters", "horizontalSpacing", "verticalSpacing"],
  );
});

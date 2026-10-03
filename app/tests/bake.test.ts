import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import { canFlatten, canMergeVisible, canRasterize, mergeKind } from "../src/lib/bake";
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

test("Rasterize applies to fills, groups and pixel layers carrying paint or effects", () => {
  assert.equal(canRasterize([layer(1)]), false);
  assert.equal(canRasterize([layer(1, { kind: "adjustment" })]), false);
  assert.equal(canRasterize([layer(1), layer(2, { painted: true })]), true);
  assert.equal(canRasterize([layer(1, { kind: "fill" })]), true);
  assert.equal(canRasterize([layer(1, { kind: "group" })]), true);
});

test("Ctrl+E merges several layers, or one down onto the visible layer below it", () => {
  const inner = layer(5);
  const group = layer(4, { kind: "group", children: [inner] });
  const hidden = layer(2, { visible: false });
  const tree = layerTree([layer(1), hidden, layer(3), group]);
  assert.equal(mergeKind(tree, [layer(1), layer(3)]), "layers");
  assert.equal(mergeKind(tree, [group]), "down");
  // Below it: hidden, or nothing.
  assert.equal(mergeKind(tree, [layer(3)]), null);
  assert.equal(mergeKind(tree, [layer(1)]), null);
  assert.equal(mergeKind(tree, [inner]), null);
  // A group and a layer inside it: the group alone.
  assert.equal(mergeKind(tree, [group, inner]), "down");
  assert.equal(mergeKind(tree, []), null);
});

test("Merge Visible needs two visible layers, or a group or a fill; Flatten more than one plain layer", () => {
  assert.equal(canMergeVisible([layer(1), layer(2, { visible: false })]), false);
  assert.equal(canMergeVisible([layer(1), layer(2)]), true);
  assert.equal(canMergeVisible([layer(1, { kind: "group" })]), true);
  assert.equal(canFlatten([layer(1)]), false);
  assert.equal(canFlatten([layer(1, { painted: true })]), true);
  assert.equal(canFlatten([layer(1), layer(2)]), true);
  assert.equal(canFlatten([]), false);
});

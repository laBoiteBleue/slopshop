import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import {
  batchOf,
  blendModeChange,
  clippingToggle,
  eyeClick,
  opacityEdit,
  opacityPercent,
  removal,
  typedOpacity,
  visibilityToggle,
} from "../src/lib/layerEdits";
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

test("one edit alone, several in one batch", () => {
  const one = { kind: "removeLayer", id: 1 } as const;
  assert.equal(batchOf([one]), one);
  assert.deepEqual(batchOf([one, one]), { kind: "batch", edits: [one, one] });
});

test("deleting a group with its layers removes the group only", () => {
  const inner = layer(3);
  const group = layer(2, { kind: "group", children: [inner] });
  const tree = layerTree([layer(1), group]);
  assert.deepEqual(removal(tree, [group, inner]), { kind: "removeLayer", id: 2 });
  assert.equal(removal(tree, []), null);
});

test("clipping clips the unclipped layers, or releases them all when all are clipped", () => {
  const mixed = [layer(1, { clipped: true }), layer(2)];
  assert.deepEqual(clippingToggle(mixed), { kind: "setLayerClipped", id: 2, clipped: true });
  const all = [layer(1, { clipped: true }), layer(2, { clipped: true })];
  assert.deepEqual(clippingToggle(all), {
    kind: "batch",
    edits: [
      { kind: "setLayerClipped", id: 1, clipped: false },
      { kind: "setLayerClipped", id: 2, clipped: false },
    ],
  });
  assert.equal(clippingToggle([]), null);
});

test("hiding the selection, or showing it all when the active layer is hidden", () => {
  const hidden = layer(1, { visible: false });
  const shown = layer(2);
  assert.deepEqual(visibilityToggle([hidden, shown], shown), {
    kind: "setLayerVisible",
    id: 2,
    visible: false,
  });
  assert.deepEqual(visibilityToggle([hidden, shown], hidden), {
    kind: "setLayerVisible",
    id: 1,
    visible: true,
  });
  assert.equal(visibilityToggle([], null), null);
});

test("an eye within a selection shows or hides it all, as the clicked layer becomes", () => {
  const [a, b, c] = [layer(1), layer(2, { visible: false }), layer(3)];
  assert.deepEqual(eyeClick(a, [a, b, c]), {
    kind: "batch",
    edits: [
      { kind: "setLayerVisible", id: 1, visible: false },
      { kind: "setLayerVisible", id: 3, visible: false },
    ],
  });
  // Outside the selection, or a selection of one: that layer alone.
  assert.deepEqual(eyeClick(b, [a, c]), { kind: "setLayerVisible", id: 2, visible: true });
  assert.deepEqual(eyeClick(a, [a]), { kind: "setLayerVisible", id: 1, visible: false });
});

test("pass through applies to groups; another mode ends it", () => {
  const group = layer(1, { kind: "group", passThrough: true });
  const raster = layer(2);
  assert.deepEqual(blendModeChange([group, raster], "multiply"), {
    kind: "batch",
    edits: [
      { kind: "setLayerBlendMode", id: 1, mode: "multiply" },
      { kind: "setGroupPassThrough", id: 1, passThrough: false },
      { kind: "setLayerBlendMode", id: 2, mode: "multiply" },
    ],
  });
  assert.equal(blendModeChange([group, raster], "passThrough"), null);
  assert.deepEqual(blendModeChange([layer(3, { kind: "group" })], "passThrough"), {
    kind: "setGroupPassThrough",
    id: 3,
    passThrough: true,
  });
});

test("the opacity field takes whole percents within range; empty or invalid restores", () => {
  assert.equal(typedOpacity("42.6", 42.6), 43);
  assert.equal(typedOpacity("150", 150), 100);
  assert.equal(typedOpacity("-5", -5), 0);
  assert.equal(typedOpacity("  ", NaN), null);
  assert.equal(typedOpacity("abc", NaN), null);
  assert.equal(opacityPercent(layer(1, { opacity: 0.555 })), 56);
  assert.equal(opacityPercent(null), 100);
  assert.deepEqual(opacityEdit([layer(1)], 0.5), { kind: "setLayerOpacity", id: 1, opacity: 0.5 });
});

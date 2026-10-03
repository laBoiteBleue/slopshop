import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import {
  batchOf,
  blendModeChange,
  canArrange,
  clippingReleases,
  clippingToggle,
  eyeClick,
  fillColorEdit,
  fillHex,
  maskEnabledToggle,
  maskRemoval,
  newFill,
  opacityEdit,
  opacityPercent,
  referenceMask,
  removal,
  typedOpacity,
  ungrouping,
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

test("the clipping command releases only when every selected layer is clipped", () => {
  assert.equal(clippingReleases([layer(1, { clipped: true }), layer(2)]), false);
  assert.equal(clippingReleases([layer(1, { clipped: true }), layer(2, { clipped: true })]), true);
  assert.equal(clippingReleases([]), false);
});

test("arranging is possible while a selected layer can pass a layer of its group", () => {
  const [a, b, c] = [layer(1), layer(2), layer(3)];
  const inner = layer(5);
  const group = layer(4, { kind: "group", children: [inner] });
  const tree = layerTree([a, b, c, group]);
  // `c` and the group are on top of the top level: up moves nothing.
  assert.equal(canArrange(tree, [c, group], "forward"), false);
  assert.equal(canArrange(tree, [c, group], "front"), false);
  assert.equal(canArrange(tree, [c, group], "backward"), true);
  assert.equal(canArrange(tree, [a], "back"), false);
  assert.equal(canArrange(tree, [a], "front"), true);
  // Alone in its group, a layer has nowhere to go; inside a moving group, it goes with it.
  assert.equal(canArrange(tree, [inner], "front"), false);
  assert.equal(canArrange(tree, [inner], "back"), false);
  assert.equal(canArrange(tree, [group, inner], "back"), true);
  assert.equal(canArrange(tree, [], "front"), false);
});

test("ungrouping replaces every selected group, their layers selected", () => {
  const inner = layer(5, { kind: "group", children: [layer(6)] });
  const outer = layer(4, { kind: "group", children: [layer(3), inner] });
  const side = layer(8, { kind: "group", children: [layer(7)] });
  assert.deepEqual(ungrouping([outer, inner, side, layer(1)]), {
    request: { kind: "ungroup", ids: [4, 5, 8] },
    // The inner group goes too: its layer, not itself, is selected.
    layers: [3, 6, 7],
  });
  assert.equal(ungrouping([layer(1)]), null);
});

test("masks are disabled, enabled and deleted on every selected layer that has one", () => {
  const on = layer(1, { mask: { enabled: true, contentKey: 1 } });
  const off = layer(2, { mask: { enabled: false, contentKey: 2 } });
  const bare = layer(3);
  // The active layer's mask says what happens; without one, the first selected mask.
  assert.equal(referenceMask([on, off, bare], bare), on.mask);
  assert.deepEqual(maskEnabledToggle([on, off, bare], on), {
    kind: "setLayerMaskEnabled",
    id: 1,
    enabled: false,
  });
  assert.deepEqual(maskEnabledToggle([on, off, bare], off), {
    kind: "setLayerMaskEnabled",
    id: 2,
    enabled: true,
  });
  assert.deepEqual(maskRemoval([on, bare, off]), {
    kind: "batch",
    edits: [
      { kind: "removeLayerMask", id: 1 },
      { kind: "removeLayerMask", id: 2 },
    ],
  });
  assert.equal(maskEnabledToggle([bare], bare), null);
  assert.equal(maskRemoval([bare]), null);
});

test("a new fill layer goes above the active layer in its group, of the color given", () => {
  const inner = layer(3);
  const tree = layerTree([layer(1), layer(2, { kind: "group", children: [inner, layer(4)] })]);
  assert.deepEqual(newFill(tree, 3, "#ff0000", "Color Fill 1"), {
    kind: "addFillLayer",
    name: "Color Fill 1",
    color: [1, 0, 0, 1],
    parent: 2,
    index: 1,
  });
  assert.deepEqual(newFill(tree, null, "#000000", "Color Fill 1"), {
    kind: "addFillLayer",
    name: "Color Fill 1",
    color: [0, 0, 0, 1],
    parent: null,
    index: 2,
  });
});

test("a fill layer's color changes only for a fill layer, and only to another color", () => {
  const fill = layer(5, { kind: "fill", swatch: [1, 0, 0, 1] });
  assert.equal(fillHex(fill), "#ff0000");
  assert.equal(fillColorEdit(fill, "#FF0000"), null);
  assert.deepEqual(fillColorEdit(fill, "#0000ff"), {
    kind: "setFillColor",
    id: 5,
    color: [0, 0, 1, 1],
  });
  assert.equal(fillColorEdit(layer(6), "#0000ff"), null);
});

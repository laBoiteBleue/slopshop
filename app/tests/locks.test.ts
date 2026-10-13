import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import { layerTree } from "../src/lib/layerTree";
import {
  commonLocks,
  effectiveLocks,
  lockMark,
  lockRefusalKey,
  lockState,
  lockToggle,
  locksEdit,
  NO_LOCKS,
} from "../src/lib/locks";

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
    locks: NO_LOCKS,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
    ...changes,
  };
}

test("a group's locks apply to the layers inside it, nested or not", () => {
  const inner = layer(3);
  const sub = layer(2, { kind: "group", children: [inner] });
  const group = layer(1, {
    kind: "group",
    locks: { ...NO_LOCKS, position: true },
    children: [sub],
  });
  const free = layer(4, { locks: { ...NO_LOCKS, transparency: true } });
  const tree = layerTree([group, free]);
  assert.deepEqual(effectiveLocks(tree, inner), { ...NO_LOCKS, position: true });
  assert.deepEqual(effectiveLocks(tree, free), { ...NO_LOCKS, transparency: true });
  assert.deepEqual(lockMark(tree, inner), { kind: "partial", inherited: true });
  assert.deepEqual(lockMark(tree, group), { kind: "partial", inherited: false });
  assert.equal(lockMark(tree, layer(9)), null);
});

test("Lock All is the full padlock, its own winning over a group's", () => {
  const child = layer(2, { locks: { ...NO_LOCKS, pixels: true } });
  const group = layer(1, { kind: "group", locks: { ...NO_LOCKS, all: true }, children: [child] });
  const tree = layerTree([group]);
  assert.deepEqual(lockMark(tree, group), { kind: "full", inherited: false });
  // Its own partial lock shows, not the group's.
  assert.deepEqual(lockMark(tree, child), { kind: "partial", inherited: false });
  assert.equal(effectiveLocks(tree, child).all, true);
});

test("a lock's button sets it on all, or clears it when all have it", () => {
  const a = layer(1, { locks: { ...NO_LOCKS, pixels: true } });
  const b = layer(2);
  assert.equal(lockState([a, b], "pixels"), "mixed");
  assert.equal(lockState([a], "pixels"), "on");
  assert.equal(lockState([b], "pixels"), "off");
  assert.deepEqual(lockToggle([a, b], "pixels"), {
    kind: "setLayerLocks",
    ids: [1, 2],
    pixels: true,
  });
  assert.deepEqual(lockToggle([a], "pixels"), { kind: "setLayerLocks", ids: [1], pixels: false });
  assert.equal(lockToggle([], "all"), null);
});

test("Lock Layers starts from the locks all the layers have and sets exactly the chosen", () => {
  const a = layer(1, { locks: { ...NO_LOCKS, pixels: true, position: true } });
  const b = layer(2, { locks: { ...NO_LOCKS, position: true } });
  assert.deepEqual(commonLocks([a, b]), { ...NO_LOCKS, position: true });
  assert.deepEqual(commonLocks([]), NO_LOCKS);
  assert.deepEqual(locksEdit([a, b], { ...NO_LOCKS, all: true }), {
    kind: "setLayerLocks",
    ids: [1, 2],
    transparency: false,
    pixels: false,
    position: false,
    all: true,
  });
  assert.equal(locksEdit([], NO_LOCKS), null);
});

test("the engine's lock refusals are recognized, other messages not", () => {
  assert.equal(lockRefusalKey("layer 3 is locked: pixels"), "locks.refused.pixels");
  assert.equal(lockRefusalKey("the layer is locked: transparency"), "locks.refused.transparency");
  assert.equal(lockRefusalKey("layer 2 is locked: position"), "locks.refused.position");
  assert.equal(lockRefusalKey("layer 2 is locked: all"), "locks.refused.all");
  assert.equal(lockRefusalKey("unknown layer 3"), null);
});

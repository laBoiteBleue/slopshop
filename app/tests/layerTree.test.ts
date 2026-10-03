import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import {
  canDropInto,
  carriesPaint,
  clipLineAt,
  dropTarget,
  findLayer,
  flattenRows,
  insertionPoint,
  layerTree,
  outermost,
  slotAt,
  topmost,
  visibleRasters,
  walk,
  within,
} from "../src/lib/layerTree";

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

const group = (id: number, children: LayerView[], changes: Partial<LayerView> = {}) =>
  layer(id, { kind: "group", children, ...changes });

/**
 * Bottom to top: background 1, group 2 holding 3 and 4 (clipped to 3), then 5. Shown top to
 * bottom: 5, 2, 4, 3, 1.
 */
const LAYERS = [layer(1), group(2, [layer(3), layer(4, { clipped: true })]), layer(5)];
const TREE = layerTree(LAYERS);
const ROWS = flattenRows(LAYERS, new Set(), []);

const ids = (layers: { id: number }[]) => layers.map((l) => l.id);

test("rows go top to bottom, each group above its layers", () => {
  assert.deepEqual(
    ROWS.map((r) => [r.layer.id, r.depth, r.parent, r.index]),
    [
      [5, 0, null, 2],
      [2, 0, null, 1],
      [4, 1, 2, 1],
      [3, 1, 2, 0],
      [1, 0, null, 0],
    ],
  );
});

test("rows mark clipped layers and their base", () => {
  assert.deepEqual(
    ROWS.map((r) => r.clipping),
    [null, null, "clipped", "base", null],
  );
  // The bottom layer of a level has nothing to be clipped to.
  const [row] = flattenRows([layer(7, { clipped: true })], new Set(), []);
  assert.equal(row.clipping, null);
});

test("folded and hidden groups leave their layers out; hidden groups hide them", () => {
  assert.deepEqual(ids(flattenRows(LAYERS, new Set([2]), []).map((r) => r.layer)), [5, 2, 1]);
  assert.deepEqual(ids(flattenRows(LAYERS, new Set(), [2]).map((r) => r.layer)), [5, 1]);
  const off = [group(2, [layer(3)], { visible: false })];
  assert.deepEqual(
    flattenRows(off, new Set(), []).map((r) => r.shown),
    [false, false],
  );
});

test("walking, finding and the visible pixel layers", () => {
  assert.deepEqual(ids(walk(LAYERS)), [1, 2, 3, 4, 5]);
  assert.equal(findLayer(LAYERS, 4)?.id, 4);
  assert.equal(findLayer(LAYERS, 9), null);
  assert.deepEqual(visibleRasters(LAYERS), [1, 3, 4, 5]);
  const off = [layer(1), group(2, [layer(3)], { visible: false }), layer(5, { visible: false })];
  assert.deepEqual(visibleRasters(off), [1]);
});

test("paint is found inside groups", () => {
  assert.equal(carriesPaint(LAYERS), false);
  assert.equal(carriesPaint([group(2, [group(6, [layer(3, { painted: true })])])]), true);
});

test("ancestry: within, outermost, topmost", () => {
  assert.equal(TREE.parents.get(4), 2);
  assert.ok(within(TREE, 4, 2));
  assert.ok(within(TREE, 2, 2));
  assert.ok(!within(TREE, 2, 4));
  assert.deepEqual(outermost(TREE, [3, 2, 5]), [2, 5]);
  assert.equal(topmost(TREE, [1, 3]), 3);
  assert.equal(topmost(TREE, []), null);
});

test("a new layer goes just above the active one, in its group, or at the top", () => {
  assert.deepEqual(insertionPoint(TREE, null), { parent: null, index: 3 });
  assert.deepEqual(insertionPoint(TREE, 3), { parent: 2, index: 1 });
  assert.deepEqual(insertionPoint(TREE, 2), { parent: null, index: 2 });
  assert.deepEqual(insertionPoint(TREE, 5), { parent: null, index: 3 });
});

test("the slot is the number of rows whose middle is above the pointer", () => {
  assert.equal(slotAt([10, 30, 50], 5), 0);
  assert.equal(slotAt([10, 30, 50], 35), 2);
  assert.equal(slotAt([10, 30, 50], 99), 3);
});

test("a drop goes just above the row of its slot, counting the layers that stay", () => {
  // Above 5, the top row: the top of the stack once 1 has left it.
  assert.deepEqual(dropTarget(TREE, ROWS, new Set([1]), null, 0), { parent: null, index: 2 });
  // Above 3, inside the group: between 3 and 4.
  assert.deepEqual(dropTarget(TREE, ROWS, new Set([5]), null, 3), { parent: 2, index: 1 });
  // Below the last row: the bottom of the stack.
  assert.deepEqual(dropTarget(TREE, ROWS, new Set([5]), null, 5), { parent: null, index: 0 });
});

test("a drop into a group goes at its top", () => {
  assert.deepEqual(dropTarget(TREE, ROWS, new Set([1]), 2, 0), { parent: 2, index: 2 });
  assert.deepEqual(dropTarget(TREE, ROWS, new Set([3]), 2, 0), { parent: 2, index: 1 });
});

test("a group cannot be dropped inside itself", () => {
  assert.equal(dropTarget(TREE, ROWS, new Set([2]), null, 3), null);
  const nested = layerTree([group(2, [group(6, [layer(3)])])]);
  const inner = findLayer(nested.layers, 6);
  assert.ok(inner);
  assert.equal(canDropInto(nested, inner, [2]), false);
  assert.equal(canDropInto(nested, inner, [3]), true);
  assert.equal(canDropInto(TREE, layer(5), [1]), false);
});

test("an Alt+click on the line between two layers of a level picks the upper one", () => {
  // Rows: 5, 2, 4, 3, 1. Near the top of row 3 (layer 3): the line between 4 and 3.
  assert.equal(clipLineAt(ROWS, 3, 2, 20)?.id, 4);
  // Near the bottom of row 2 (layer 4): the same line.
  assert.equal(clipLineAt(ROWS, 2, 20, 2)?.id, 4);
  // In the middle of a row: no line.
  assert.equal(clipLineAt(ROWS, 3, 10, 10), null);
  // Between a group and its first layer, or past the last row: no.
  assert.equal(clipLineAt(ROWS, 2, 2, 20), null);
  assert.equal(clipLineAt(ROWS, 4, 20, 2), null);
  assert.equal(clipLineAt(ROWS, 0, 2, 20), null);
});

test("a group being baked is listed alone, as the layer it becomes", () => {
  const inner = layer(2);
  const baking = layer(1, { kind: "group", children: [inner], baking: true });
  assert.deepEqual(
    flattenRows([baking], new Set(), []).map((r) => r.layer.id),
    [1],
  );
});

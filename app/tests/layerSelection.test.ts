import { test } from "vitest";
import assert from "node:assert/strict";
import {
  NO_LAYERS,
  afterLayersChange,
  allSelected,
  pickedInImage,
  pressed,
  ranged,
  selectionOf,
  toggled,
} from "../src/lib/layerSelection";

/** Every layer, bottom to top. */
const ORDER = [1, 2, 3, 4, 5];
/** The rows, top to bottom. */
const ROWS = [5, 4, 3, 2, 1];

test("a new panel selects the top layer", () => {
  assert.deepEqual(afterLayersChange(NO_LAYERS, new Set(), ORDER), selectionOf([5], 5));
  assert.deepEqual(afterLayersChange(NO_LAYERS, new Set(), []), selectionOf([], null));
});

test("new layers become the selection, the topmost active", () => {
  const before = selectionOf([2], 2);
  assert.deepEqual(
    afterLayersChange(before, new Set(ORDER), [1, 2, 6, 7, 3, 4, 5]),
    selectionOf([6, 7], 7),
  );
});

test("deleted layers leave the selection; the active one moves to the topmost left", () => {
  const before = { ids: [2, 4, 3], active: 4, anchor: 4 };
  assert.deepEqual(afterLayersChange(before, new Set(ORDER), [1, 2, 3, 5]), {
    ids: [2, 3],
    active: 3,
    anchor: 3,
  });
  // The anchor stays when it is still there.
  const anchored = { ids: [2, 4], active: 4, anchor: 2 };
  assert.deepEqual(afterLayersChange(anchored, new Set(ORDER), [1, 2, 3, 5]), {
    ids: [2],
    active: 2,
    anchor: 2,
  });
});

test("when every selected layer is deleted, the top layer is selected", () => {
  assert.deepEqual(
    afterLayersChange(selectionOf([4], 4), new Set(ORDER), [1, 2, 3, 5]),
    selectionOf([5], 5),
  );
});

test("an empty selection stays empty, and an unchanged one is the same object", () => {
  assert.equal(afterLayersChange(NO_LAYERS, new Set(ORDER), ORDER), NO_LAYERS);
  const selection = selectionOf([2, 3], 3);
  assert.equal(afterLayersChange(selection, new Set(ORDER), ORDER), selection);
});

test("Ctrl+click adds a layer and makes it active, or removes it", () => {
  assert.deepEqual(toggled(selectionOf([2], 2), 4, ORDER), selectionOf([2, 4], 4));
  // Removing the active layer: the topmost left becomes active.
  assert.deepEqual(toggled(selectionOf([4, 2, 3], 2), 2, ORDER), selectionOf([4, 3], 4));
  assert.deepEqual(toggled(selectionOf([4, 2], 2), 4, ORDER), selectionOf([2], 2));
  assert.deepEqual(toggled(selectionOf([2], 2), 2, ORDER), selectionOf([], null));
});

test("Shift+click selects the rows from the anchor, which stays", () => {
  const range = ranged(selectionOf([4], 4), 2, ROWS);
  assert.deepEqual(range, { ids: [4, 3, 2], active: 2, anchor: 4 });
  // From the same anchor, the other way.
  assert.deepEqual(ranged(range, 5, ROWS), { ids: [5, 4], active: 5, anchor: 4 });
  // Without an anchor, the layer alone.
  assert.deepEqual(ranged(NO_LAYERS, 3, ROWS), { ids: [3], active: 3, anchor: null });
  // An anchor no longer shown (in a folded group): the layer alone.
  assert.deepEqual(ranged(selectionOf([9], 9), 3, ROWS), selectionOf([3], 3));
});

test("Select All Layers keeps the active layer", () => {
  assert.deepEqual(allSelected(selectionOf([2], 2), ORDER), selectionOf(ORDER, 2));
  assert.deepEqual(allSelected(NO_LAYERS, ORDER), selectionOf(ORDER, 5));
});

test("a press within a selection makes the layer active and keeps the selection", () => {
  assert.deepEqual(pressed(selectionOf([2, 4], 4), 2), {
    selection: { ids: [2, 4], active: 2, anchor: 2 },
    collapse: true,
  });
  assert.deepEqual(pressed(selectionOf([2, 4], 4), 3), {
    selection: selectionOf([3], 3),
    collapse: false,
  });
  assert.deepEqual(pressed(selectionOf([2], 2), 2), {
    selection: selectionOf([2], 2),
    collapse: false,
  });
});

test("a press on a layer in the image selects it alone, as a press on its row", () => {
  const picked = pickedInImage(selectionOf([2], 2), 4, false, ORDER);
  assert.deepEqual(picked, { selection: selectionOf([4], 4), collapse: false, moves: true });
  // On a layer of a multiple selection: all kept for a drag, that one alone on a click.
  const several = selectionOf([2, 4, 3], 3);
  assert.deepEqual(pickedInImage(several, 4, false, ORDER), {
    selection: { ids: [2, 4, 3], active: 4, anchor: 4 },
    collapse: true,
    moves: true,
  });
});

test("Shift+press in the image adds the layer, active, or takes it out without moving", () => {
  const added = pickedInImage(selectionOf([2], 2), 4, true, ORDER);
  assert.deepEqual(added, { selection: selectionOf([2, 4], 4), collapse: false, moves: true });
  const removed = pickedInImage(selectionOf([2, 4], 4), 4, true, ORDER);
  assert.deepEqual(removed, { selection: selectionOf([2], 2), collapse: false, moves: false });
});

test("a press where no layer shows keeps the selection, and moves it but with Shift", () => {
  const selection = selectionOf([2, 3], 3);
  assert.deepEqual(pickedInImage(selection, null, false, ORDER), {
    selection,
    collapse: false,
    moves: true,
  });
  assert.equal(pickedInImage(selection, null, true, ORDER).moves, false);
});

import { test } from "vitest";
import assert from "node:assert/strict";
import { cycled, moveTab, shared, tabSlot, upsert } from "../src/lib/tabs";

test("a document view adds its tab, or updates it unless stale", () => {
  const tabs = [
    { id: 1, revision: 3, name: "a" },
    { id: 2, revision: 1, name: "b" },
  ];
  upsert(tabs, { id: 3, revision: 0, name: "c" });
  upsert(tabs, { id: 1, revision: 2, name: "old" });
  upsert(tabs, { id: 2, revision: 1, name: "same revision" });
  assert.deepEqual(
    tabs.map((d) => d.name),
    ["a", "same revision", "c"],
  );
});

test("Ctrl+Tab and Ctrl+Shift+Tab go round", () => {
  assert.equal(cycled([1, 2, 3], 3, 1), 1);
  assert.equal(cycled([1, 2, 3], 1, -1), 3);
  assert.equal(cycled([1, 2, 3], 2, 1), 3);
  assert.equal(cycled([1], 1, 1), null);
  // No active tab: the first, or the last going back.
  assert.equal(cycled([1, 2, 3], null, 1), 1);
  assert.equal(cycled([1, 2, 3], null, -1), 2);
});

test("a dragged tab lands between the tabs around the pointer, not next to itself", () => {
  const middles = [50, 150, 250, 350];
  assert.equal(tabSlot(middles, 10, 2), 0);
  assert.equal(tabSlot(middles, 400, 0), 4);
  // Right before or after itself: no move.
  assert.equal(tabSlot(middles, 200, 2), null);
  assert.equal(tabSlot(middles, 300, 2), null);
});

test("a moved tab's index counts the other tabs", () => {
  const tabs = ["a", "b", "c", "d"];
  assert.equal(moveTab(tabs, 0, 3), 2);
  assert.deepEqual(tabs, ["b", "c", "a", "d"]);
  assert.equal(moveTab(tabs, 3, 0), 0);
  assert.deepEqual(tabs, ["d", "b", "c", "a"]);
  assert.equal(moveTab(tabs, 1, 4), 3);
  assert.deepEqual(tabs, ["d", "c", "a", "b"]);
});

test("a view keeps the parts that did not change, so that what shows them is not run again", () => {
  const layer = (id: number, name: string) => ({ id, name, entries: [{ kind: "levels" }] });
  const tabs = [{ id: 1, revision: 1, layers: [layer(7, "a"), layer(8, "b")], path: "x" }];
  const before = tabs[0];
  const [a, b] = before.layers;
  // A slider dragged over layer 8: the whole document comes back.
  upsert(tabs, { id: 1, revision: 2, layers: [layer(7, "a"), layer(8, "b2")], path: "x" });
  const after = tabs[0];
  assert.notEqual(after, before);
  assert.equal(after.revision, 2);
  assert.equal(after.layers[0], a);
  assert.notEqual(after.layers[1], b);
  assert.equal(after.layers[1].name, "b2");
  assert.equal(after.layers[1].entries, b.entries);
  // The view before is left as it was.
  assert.equal(before.revision, 1);
  assert.equal(before.layers[1].name, "b");
});

test("shared parts follow ids, keys that went away go, and equal views stay the same", () => {
  const view = {
    layers: [
      { id: 1, v: [1, 2] },
      { id: 2, v: [3] },
    ],
    note: "n",
  };
  // Reordered: each layer keeps its own; the array is new.
  const reordered = shared(view, {
    layers: [
      { id: 2, v: [3] },
      { id: 1, v: [1, 2] },
    ],
    note: "n",
  });
  assert.equal(reordered.layers[0], view.layers[1]);
  assert.equal(reordered.layers[1], view.layers[0]);
  assert.notEqual(reordered.layers, view.layers);
  // A key gone and a layer removed.
  const fewer = shared<Record<string, unknown>>(view, { layers: [{ id: 1, v: [1, 2] }] });
  assert.deepEqual(Object.keys(fewer), ["layers"]);
  assert.deepEqual(fewer.layers, [{ id: 1, v: [1, 2] }]);
  // Equal: the same object.
  assert.equal(shared(view, structuredClone(view)), view);
  // Values of other kinds replace.
  assert.equal(shared<unknown>({ a: 1 }, null), null);
  assert.deepEqual(shared<unknown>([1, 2], { a: 1 }), { a: 1 });
});

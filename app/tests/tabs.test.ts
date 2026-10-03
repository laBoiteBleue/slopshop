import { test } from "node:test";
import assert from "node:assert/strict";
import { cycled, moveTab, tabSlot, upsert } from "../src/lib/tabs.ts";

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

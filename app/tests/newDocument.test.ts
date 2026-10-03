import { test } from "node:test";
import assert from "node:assert/strict";
import { atResolution, isValidNew, matchingPreset, oriented } from "../src/lib/newDocument.ts";

const PRESETS = [
  { id: "hd", width: 1920, height: 1080 },
  { id: "a4", width: 2480, height: 3508 },
];

test("a size matches its preset either way round, else it is custom", () => {
  assert.equal(matchingPreset(PRESETS, { width: 1920, height: 1080 }), "hd");
  assert.equal(matchingPreset(PRESETS, { width: 3508, height: 2480 }), "a4");
  assert.equal(matchingPreset(PRESETS, { width: 1920, height: 1081 }), "custom");
});

test("a size typed in a length keeps it when the resolution changes; in pixels it stays", () => {
  assert.deepEqual(atResolution({ width: 600, height: 300 }, "cm", 300, 150), {
    width: 300,
    height: 150,
  });
  assert.deepEqual(atResolution({ width: 600, height: 300 }, "px", 300, 150), {
    width: 600,
    height: 300,
  });
  // Whole pixels.
  assert.deepEqual(atResolution({ width: 100, height: 100 }, "in", 300, 72), {
    width: 24,
    height: 24,
  });
});

test("portrait and landscape swap the sides when needed; a square stays", () => {
  assert.deepEqual(oriented({ width: 4, height: 3 }, true), { width: 3, height: 4 });
  assert.deepEqual(oriented({ width: 3, height: 4 }, true), { width: 3, height: 4 });
  assert.deepEqual(oriented({ width: 3, height: 4 }, false), { width: 4, height: 3 });
  assert.deepEqual(oriented({ width: 5, height: 5 }, true), { width: 5, height: 5 });
});

test("a new document needs whole pixels within range and an accepted resolution", () => {
  assert.ok(isValidNew({ width: 1, height: 300_000 }, 72));
  assert.ok(!isValidNew({ width: 0, height: 10 }, 72));
  assert.ok(!isValidNew({ width: 10.5, height: 10 }, 72));
  assert.ok(!isValidNew({ width: 10, height: 300_001 }, 72));
  assert.ok(!isValidNew({ width: 10, height: 10 }, 0));
});

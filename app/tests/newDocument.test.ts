import { test } from "vitest";
import assert from "node:assert/strict";
import { atResolution, isValidNew, matchingPreset, oriented } from "../src/lib/newDocument";

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
  const size = { width: 2480, height: 3508 };
  // A4 shown as 21 × 29.7 cm.
  const lengths = { width: 21, height: 29.7 };
  assert.deepEqual(atResolution(size, lengths, "cm", 150), { width: 1240, height: 1754 });
  assert.deepEqual(atResolution(size, lengths, "px", 150), size);
  assert.deepEqual(atResolution(size, { width: NaN, height: 1 }, "cm", 150), size);
});

test("a resolution typed digit by digit does not round the size away", () => {
  // Regression: the pixels were scaled from the previous ones at each keystroke (1, 15, 150).
  const lengths = { width: 21, height: 29.7 };
  let size = { width: 2480, height: 3508 };
  for (const ppi of [1, 15, 150]) size = atResolution(size, lengths, "cm", ppi);
  assert.deepEqual(size, { width: 1240, height: 1754 });
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

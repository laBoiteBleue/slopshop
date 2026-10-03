import { test } from "node:test";
import assert from "node:assert/strict";
import { outline } from "../src/lib/contour.ts";

const mask = (rows: string[]) => Uint8Array.from(rows.join(""), (c) => (c === "#" ? 1 : 0));

test("an empty mask has no outline", () => {
  assert.deepEqual(outline(mask(["...", "...", "..."]), 3), []);
});

test("a single cell is a diamond through its sides' midpoints", () => {
  const loops = outline(mask(["#"]), 1, 0);
  assert.equal(loops.length, 1);
  const points = loops[0].map(([x, y]) => `${x},${y}`).sort();
  assert.deepEqual(points, ["0,0.5", "0.5,0", "0.5,1", "1,0.5"]);
});

test("separate shapes, holes and diagonal neighbors give separate loops", () => {
  assert.equal(outline(mask(["#..", "...", "..#"]), 3).length, 2);
  assert.equal(outline(mask(["###", "#.#", "###"]), 3).length, 2);
  // A saddle: the two inside cells only touch at a corner and stay apart.
  assert.equal(outline(mask(["#.", ".#"]), 2).length, 2);
  assert.equal(outline(mask(["##", "##"]), 2).length, 1);
});

test("each round of smoothing doubles the points, within the mask's extent", () => {
  const [loop] = outline(mask(["#"]), 1, 2);
  assert.equal(loop.length, 16);
  assert.ok(loop.every(([x, y]) => x >= 0 && x <= 1 && y >= 0 && y <= 1));
});

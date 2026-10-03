import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import { landing, nudged, pixelTarget } from "../src/lib/moveTool";

const layer = (changes: Partial<LayerView>) =>
  ({ id: 7, kind: "raster", visible: true, ...changes }) as LayerView;

test("selected pixels move from a visible pixel layer, or from a targeted mask", () => {
  assert.deepEqual(pixelTarget(layer({}), false), { target: "layer", layerId: 7 });
  assert.deepEqual(pixelTarget(layer({ kind: "fill" }), true), { target: "mask", layerId: 7 });
  assert.deepEqual(pixelTarget(layer({ kind: "fill" }), false), { error: "move.needRaster" });
  assert.deepEqual(pixelTarget(null, false), { error: "move.needRaster" });
  assert.deepEqual(pixelTarget(layer({ visible: false }), false), { error: "move.hidden" });
});

test("arrows move the outline with a selection tool, the pixels with the Move tool", () => {
  assert.equal(nudged("marquee", true), "outline");
  assert.equal(nudged("quickSelection", true), "outline");
  assert.equal(nudged("move", true), "pixels");
  assert.equal(nudged("brush", true), "layers");
  // Without a selection (or in Quick Mask), the layers.
  assert.equal(nudged("move", false), "layers");
  assert.equal(nudged("marquee", false), "layers");
});

test("a drag lands on whole pixels, snapped when what moves is known", () => {
  const moving = { left: 0, top: 0, right: 100, bottom: 100 };
  const canvas = { left: 0, top: 0, right: 1000, bottom: 1000 };
  assert.deepEqual(landing({ x: 3.4, y: -2.6 }, null, [canvas], 6), { x: 3, y: -3, guides: [] });
  const snapped = landing({ x: 897.2, y: 40.4 }, moving, [canvas], 6);
  assert.equal(snapped.x, 900);
  assert.equal(snapped.y, 40);
  assert.equal(snapped.guides.length, 1);
});

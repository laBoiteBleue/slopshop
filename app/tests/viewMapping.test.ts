import { test } from "vitest";
import assert from "node:assert/strict";
import {
  NO_VIEW,
  PIXEL_GRID_ZOOM,
  easeStep,
  pixelGrid,
  reprojection,
  toDocument,
  toViewport,
  wheelPixels,
  type View,
} from "../src/lib/viewMapping";

const closeAll = (actual: readonly number[], expected: readonly number[]) =>
  assert.ok(
    actual.every((v, i) => Math.abs(v - expected[i]) < 1e-9),
    `[${actual}] ≠ [${expected}]`,
  );

test("viewport and document points map both ways, with the display's scale", () => {
  const view: View = { zoom: 3, origin: [100, 50] };
  // At 150% display scale, a CSS pixel is 1.5 device pixels; a document pixel is 3.
  closeAll(toDocument(view, 1.5, 20, 40), [110, 70]);
  closeAll(toViewport(view, 1.5, 110, 70), [20, 40]);
  closeAll(toViewport(view, 1.5, ...toDocument(view, 1.5, 7, 9)), [7, 9]);
  closeAll(toDocument(NO_VIEW, 1, 7, 9), [7, 9]);
});

test("a frame drawn for the shown view moves and scales to the target's", () => {
  const shown: View = { zoom: 1, origin: [0, 0] };
  assert.equal(reprojection(shown, shown, 1), null);
  // Zoomed in twice: the frame doubles.
  assert.deepEqual(reprojection(shown, { zoom: 2, origin: [0, 0] }, 1), { tx: 0, ty: 0, k: 2 });
  // Panned right by 10 document pixels: the frame goes left by 10 × zoom / dpr CSS pixels.
  assert.deepEqual(reprojection(shown, { zoom: 2, origin: [10, 0] }, 2), {
    tx: -10,
    ty: 0,
    k: 2,
  });
  // A point drawn in the frame lands where the target view puts it.
  const target: View = { zoom: 3, origin: [4, -2] };
  const r = reprojection(shown, target, 1);
  assert.ok(r);
  const [x, y] = [5, 8]; // output pixels of the shown frame (document 5, 8)
  closeAll([r.tx + x * r.k, r.ty + y * r.k], toViewport(target, 1, 5, 8));
});

test("the smooth zoom eases towards the target and finishes exactly", () => {
  const step = easeStep(1, 50, 50);
  closeAll([step], [1 - Math.exp(-1)]);
  assert.equal(easeStep(1, 0, 50), 0);
  // Almost there: the rest at once, so the animation ends.
  assert.equal(easeStep(0.001, 1, 50), 0.001);
  assert.equal(easeStep(-0.001, 1, 50), -0.001);
});

test("wheel deltas in pixels, lines or pages", () => {
  assert.equal(wheelPixels(120, 0, 800), 120);
  assert.equal(wheelPixels(3, 1, 800), 48);
  assert.equal(wheelPixels(-1, 2, 800), -800);
});

test("the pixel grid shows from 800%, over the visible part of the canvas only", () => {
  const canvas = { width: 100, height: 50 };
  assert.equal(pixelGrid({ zoom: 4, origin: [0, 0] }, 1, canvas, 400, 300), null);
  // 800%, the document's (10, 5) at the viewport's corner, on a 2× display: 4 CSS pixels a cell.
  const view: View = { zoom: PIXEL_GRID_ZOOM, origin: [10, 5] };
  const grid = pixelGrid(view, 2, canvas, 300, 100);
  // The canvas starts 10 × 4 and 5 × 4 CSS pixels before the corner, and goes past the
  // viewport's other sides.
  assert.deepEqual(grid, {
    left: 0,
    top: 0,
    width: 300,
    height: 100,
    cell: 4,
    offsetX: -40,
    offsetY: -20,
  });
  // Scrolled away from the canvas: nothing to draw.
  assert.equal(pixelGrid({ zoom: 8, origin: [500, 0] }, 1, canvas, 300, 100), null);
});

import { test } from "vitest";
import assert from "node:assert/strict";
import { FIT_MARGIN, layoutOf, type PrintSettings } from "../src/lib/print";

const settings = (changes: Partial<PrintSettings>): PrintSettings => ({
  paper: "a4",
  landscape: false,
  fit: false,
  ppi: 300,
  center: true,
  left: 0,
  top: 0,
  ...changes,
});

const close = (actual: number, expected: number) =>
  assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} ≠ ${expected}`);

test("Scale to Fit fills the paper within its margin, centered", () => {
  const layout = layoutOf(settings({ fit: true }), 1000, 1000);
  close(layout.width, 210 - 2 * FIT_MARGIN);
  close(layout.height, 210 - 2 * FIT_MARGIN);
  close(layout.left, FIT_MARGIN);
  close(layout.top, (297 - 190) / 2);
  close(layout.ppi, 25.4 / 0.19);
});

test("landscape turns the paper", () => {
  const layout = layoutOf(settings({ fit: true, landscape: true }), 2000, 1000);
  assert.equal(layout.pageWidth, 297);
  assert.equal(layout.pageHeight, 210);
  close(layout.width, 277);
});

test("at a resolution, the image keeps its printed size where it is placed", () => {
  const layout = layoutOf(settings({ ppi: 254, center: false, left: 12, top: 34 }), 1000, 500);
  close(layout.width, 100);
  close(layout.height, 50);
  assert.equal(layout.left, 12);
  assert.equal(layout.top, 34);
  close(layout.ppi, 254);
  const centered = layoutOf(settings({ ppi: 254 }), 1000, 500);
  close(centered.left, 55);
});

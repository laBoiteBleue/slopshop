import { test } from "vitest";
import assert from "node:assert/strict";
import { LENGTH_UNITS, fromPixels, fromPpi, rounded, toPixels, toPpi } from "../src/lib/units";

const close = (actual: number, expected: number) =>
  assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} ≠ ${expected}`);

test("pixels as inches, centimeters and millimeters at the document's resolution", () => {
  close(fromPixels(300, "in", 300), 1);
  close(fromPixels(300, "cm", 300), 2.54);
  close(fromPixels(300, "mm", 300), 25.4);
  assert.equal(fromPixels(300, "px", 72), 300);
});

test("lengths come back to the same pixels in every unit", () => {
  for (const unit of LENGTH_UNITS) {
    for (const ppi of [72, 96, 300]) {
      close(toPixels(fromPixels(1234, unit, ppi), unit, ppi), 1234);
    }
  }
});

test("resolutions in pixels per centimeter", () => {
  close(fromPpi(254, "ppcm"), 100);
  close(toPpi(100, "ppcm"), 254);
  assert.equal(fromPpi(300, "ppi"), 300);
  assert.equal(toPpi(300, "ppi"), 300);
});

test("fields are rounded as Photoshop shows each unit", () => {
  assert.equal(rounded(1.23456, "px"), 1);
  assert.equal(rounded(1.23456, "in"), 1.235);
  assert.equal(rounded(1.23456, "cm"), 1.23);
  assert.equal(rounded(1.23456, "mm"), 1.2);
  assert.equal(rounded(1.23456, "ppcm"), 1.23);
  assert.equal(rounded(12.345, "percent"), 12.3);
});

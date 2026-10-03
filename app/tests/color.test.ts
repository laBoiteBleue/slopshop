import { test } from "node:test";
import assert from "node:assert/strict";
import { hexToSrgb, srgbToHex } from "../src/lib/color.ts";

test("color inputs' values and sRGB components convert both ways", () => {
  assert.deepEqual(hexToSrgb("#ff0000"), [1, 0, 0]);
  assert.deepEqual(hexToSrgb("#000000"), [0, 0, 0]);
  for (const hex of ["#000000", "#ffffff", "#12ab9f", "#808080"]) {
    assert.equal(srgbToHex(hexToSrgb(hex)), hex);
  }
});

test("components out of range are clamped, and alpha is ignored", () => {
  assert.equal(srgbToHex([1.5, -0.2, 0.5, 0.3]), "#ff0080");
});

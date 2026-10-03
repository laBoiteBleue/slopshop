import { test } from "node:test";
import assert from "node:assert/strict";
import {
  hexToRgb,
  hsbToRgb,
  labToRgb,
  rgbToHex,
  rgbToHsb,
  rgbToLab,
  type Rgb,
} from "../src/lib/colorModel.ts";

const closeAll = (actual: readonly number[], expected: readonly number[], tolerance = 1e-9) =>
  assert.ok(
    actual.every((v, i) => Math.abs(v - expected[i]) < tolerance),
    `[${actual}] ≠ [${expected}]`,
  );

test("HSB primaries and back", () => {
  closeAll(hsbToRgb([0, 1, 1]), [1, 0, 0]);
  closeAll(hsbToRgb([120, 1, 1]), [0, 1, 0]);
  closeAll(hsbToRgb([240, 1, 1]), [0, 0, 1]);
  for (const hsb of [
    [30, 0.5, 0.8],
    [200, 1, 0.25],
    [330, 0.1, 1],
  ] as const) {
    closeAll(rgbToHsb(hsbToRgb([...hsb])), hsb);
  }
});

test("a gray keeps the hue it is given, so that the hue slider does not jump", () => {
  closeAll(rgbToHsb([0.5, 0.5, 0.5], 200), [200, 0, 0.5]);
  closeAll(rgbToHsb([0, 0, 0], 42), [42, 0, 0]);
});

test("Lab: white and black at D50, and colors come back", () => {
  // The ICC matrix's white is D50 to 4 decimals: a and b are off by a few hundredths.
  closeAll(rgbToLab([1, 1, 1]), [100, 0, 0], 0.05);
  closeAll(rgbToLab([0, 0, 0]), [0, 0, 0], 1e-9);
  for (const rgb of [
    [1, 0, 0],
    [0.2, 0.6, 0.4],
    [0.9, 0.85, 0.1],
    [0.01, 0.02, 0.03],
  ] as Rgb[]) {
    const back = labToRgb(rgbToLab(rgb));
    assert.equal(back.clipped, false);
    closeAll(back.rgb, rgb, 1e-5);
  }
});

test("a Lab color outside sRGB is clipped, and says so", () => {
  const { rgb, clipped } = labToRgb([50, 120, 0]);
  assert.equal(clipped, true);
  assert.ok(rgb.every((c) => c >= 0 && c <= 1));
});

test("hex in and out", () => {
  assert.equal(rgbToHex([1, 0, 0.5]), "#ff0080");
  assert.equal(rgbToHex([2, -1, 0]), "#ff0000");
  closeAll(hexToRgb("#f00") ?? [], [1, 0, 0]);
  closeAll(hexToRgb(" 00ff80 ") ?? [], [0, 1, 128 / 255]);
  assert.equal(hexToRgb("#12345"), null);
  assert.equal(hexToRgb("zzzzzz"), null);
});

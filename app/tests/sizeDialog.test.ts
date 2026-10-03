import { test } from "vitest";
import assert from "node:assert/strict";
import {
  MAX_SIDE,
  initialSize,
  isLocked,
  isValid,
  newSize,
  sideField,
  typeResolution,
  typeSide,
  withResample,
  type SizeState,
} from "../src/lib/sizeDialog";

/** 100 × 200 pixels at 254 ppi: 100 pixels per centimeter. */
const image = (changes: Partial<SizeState> = {}): SizeState => ({
  ...initialSize("image", { width: 100, height: 200, ppi: 254 }),
  ...changes,
});
const canvas = (changes: Partial<SizeState> = {}): SizeState => ({
  ...initialSize("canvas", { width: 100, height: 200, ppi: 254 }),
  ...changes,
});

const close = (actual: number, expected: number) =>
  assert.ok(Math.abs(actual - expected) < 1e-9, `${actual} ≠ ${expected}`);

test("Image Size keeps the proportions: the other side follows and is rewritten", () => {
  const change = typeSide(image(), "width", 50);
  assert.ok(change);
  assert.deepEqual(newSize(change.state), { width: 50, height: 100 });
  assert.deepEqual(change.rewrite, ["height"]);
  const unlinked = typeSide(image({ constrain: false }), "height", 50);
  assert.deepEqual(unlinked && newSize(unlinked.state), { width: 100, height: 50 });
  assert.deepEqual(unlinked?.rewrite, []);
});

test("sides in percent and in lengths at the resolution", () => {
  const half = typeSide(image({ unit: "percent", constrain: false }), "width", 50);
  assert.equal(half?.state.width, 50);
  const cm = typeSide(image({ unit: "cm", constrain: false }), "width", 3);
  close(cm?.state.width ?? NaN, 300);
  assert.equal(sideField(image({ unit: "cm" }), "height"), 2);
  assert.equal(sideField(image({ unit: "percent" }), "width"), 100);
});

test("Canvas Size's relative sizes add to the current ones, and never link", () => {
  const state = canvas({ relative: true });
  assert.equal(sideField(state, "width"), 0);
  const change = typeSide(state, "width", 20);
  assert.ok(change);
  assert.deepEqual(newSize(change.state), { width: 120, height: 200 });
  assert.deepEqual(change.rewrite, []);
  assert.equal(sideField(change.state, "width"), 20);
  const shrunk = typeSide(canvas({ relative: true, unit: "percent" }), "height", -50);
  assert.equal(shrunk?.state.height, 100);
});

test("without resampling, a printed size sets the resolution and the pixels stay", () => {
  const state = withResample(image(), false);
  assert.equal(state.unit, "cm");
  assert.ok(isLocked({ ...state, unit: "px" }));
  assert.ok(!isLocked(state));
  // 2 cm wide for 100 pixels: 50 pixels per centimeter, 127 per inch.
  const change = typeSide(state, "width", 2);
  assert.ok(change);
  close(change.state.ppi, 127);
  assert.deepEqual(newSize(change.state), { width: 100, height: 200 });
  assert.deepEqual(change.rewrite, ["resolution", "height"]);
  assert.equal(sideField(change.state, "height"), 4);
  assert.equal(typeSide(state, "width", 0), null);
});

test("turning Resample off goes back to the image's pixels", () => {
  const typed = typeSide(image({ unit: "percent" }), "width", 50);
  assert.ok(typed);
  const off = withResample(typed.state, false);
  assert.deepEqual(newSize(off), { width: 100, height: 200 });
  assert.equal(off.unit, "cm");
  assert.equal(withResample(image({ unit: "mm" }), false).unit, "mm");
});

test("a resolution typed resamples the pixels, or changes the printed size", () => {
  // Resampling: the print size stays, the pixels double; pixel fields are rewritten.
  const resampled = typeResolution(image(), 508);
  assert.ok(resampled);
  assert.deepEqual(newSize(resampled.state), { width: 200, height: 400 });
  assert.deepEqual(resampled.rewrite, ["width", "height"]);
  // In centimeters the fields stay (the print size did not change).
  assert.deepEqual(typeResolution(image({ unit: "cm" }), 508)?.rewrite, []);
  // Without resampling the pixels stay; the printed size changes.
  const kept = typeResolution(withResample(image(), false), 508);
  assert.ok(kept);
  assert.deepEqual(newSize(kept.state), { width: 100, height: 200 });
  assert.deepEqual(kept.rewrite, ["width", "height"]);
  assert.equal(typeResolution(image(), 0), null);
  assert.equal(typeResolution(image(), NaN), null);
});

test("sizes and resolutions out of range are not valid", () => {
  assert.ok(isValid(image()));
  assert.ok(!isValid(image({ width: 0.4 })));
  assert.ok(!isValid(image({ height: MAX_SIDE + 1 })));
  assert.ok(!isValid(image({ ppi: 0.5 })));
  assert.ok(!isValid(image({ ppi: NaN })));
});

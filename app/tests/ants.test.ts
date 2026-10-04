import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { antsRequest, prefersReducedMotion, type AntsContext } from "../src/lib/ants";

const shown: AntsContext = {
  native: true,
  selected: true,
  hidden: false,
  reducedMotion: false,
};

test("the engine draws the ants of a selection in the native view, marching", () => {
  assert.deepEqual(antsRequest(shown), { matrix: [1, 0, 0, 1, 0, 0], march: true });
});

test("nothing is asked where the UI draws them (frames), or without a selection", () => {
  assert.equal(antsRequest({ ...shown, native: false }), null);
  assert.equal(antsRequest({ ...shown, selected: false }), null);
});

test("nothing is asked while the selection is shown another way (Quick Mask, Select and Mask)", () => {
  assert.equal(antsRequest({ ...shown, hidden: true }), null);
});

test("reduced motion: the dashes stand still", () => {
  assert.equal(antsRequest({ ...shown, reducedMotion: true })?.march, false);
});

test("a drag's shift moves the outline, Transform Selection's matrix maps it", () => {
  assert.deepEqual(antsRequest({ ...shown, shift: [30, -20] })?.matrix, [1, 0, 0, 1, 30, -20]);
  assert.deepEqual(antsRequest({ ...shown, shift: [0, 0] })?.matrix, [1, 0, 0, 1, 0, 0]);
  const flip: [number, number, number, number, number, number] = [-1, 0, 0, 1, 120, 0];
  assert.deepEqual(antsRequest({ ...shown, matrix: flip })?.matrix, flip);
  // Both: mapped first, then moved.
  assert.deepEqual(
    antsRequest({ ...shown, matrix: flip, shift: [5, 7] })?.matrix,
    [-1, 0, 0, 1, 125, 7],
  );
});

test("reduced motion is what the system's media query says", () => {
  const query = (matches: boolean) =>
    vi.stubGlobal("matchMedia", (media: string) => ({
      matches: matches && media === "(prefers-reduced-motion: reduce)",
    }));
  query(true);
  assert.equal(prefersReducedMotion(), true);
  query(false);
  assert.equal(prefersReducedMotion(), false);
  vi.unstubAllGlobals();
});

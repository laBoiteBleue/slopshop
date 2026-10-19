import { afterEach, expect, test, vi } from "vitest";
import type { ContourPreset, StyleContour } from "../src/lib/engine";
import {
  CONTOUR_PRESET_IDS,
  contourPath,
  contourPresets,
  forgetContourPresets,
  presetOf,
  samplesOf,
  sameContour,
} from "../src/lib/styleContours";

const ask = vi.hoisted(() => vi.fn());
vi.mock("../src/lib/engine", () => ({ engine: { contourPresets: ask } }));

const linear: StyleContour = {
  points: [
    [0, 0, false],
    [255, 255, false],
  ],
  samples: [0, 0.5, 1],
};
const cone: StyleContour = {
  points: [
    [0, 0, false],
    [128, 255, true],
    [255, 0, false],
  ],
  samples: [0, 1, 0],
};
const presets: ContourPreset[] = [
  { id: "linear", contour: linear },
  { id: "cone", contour: cone },
];

afterEach(() => {
  ask.mockReset();
  forgetContourPresets();
});

test("a contour is the preset through the same points, else custom; absent, the default", () => {
  expect(sameContour(cone, { points: structuredClone(cone.points) })).toBe(true);
  // A corner differs.
  const smooth: StyleContour = {
    points: [
      [0, 0, false],
      [128, 255, false],
      [255, 0, false],
    ],
  };
  expect(sameContour(cone, smooth)).toBe(false);
  expect(presetOf({ points: cone.points }, "linear", presets)).toBe("cone");
  expect(presetOf(smooth, "linear", presets)).toBeNull();
  expect(presetOf(undefined, "gaussian", presets)).toBe("gaussian");
  // The presets not loaded yet: a contour sent is custom until they are.
  expect(presetOf(cone, "linear", [])).toBeNull();
});

test("a contour's thumbnail is drawn from its samples, else its preset's", () => {
  expect(samplesOf(cone, "linear", presets)).toEqual([0, 1, 0]);
  expect(samplesOf({ points: cone.points }, "linear", presets)).toEqual([0, 1, 0]);
  expect(samplesOf(undefined, "linear", presets)).toEqual([0, 0.5, 1]);
  expect(samplesOf(undefined, "gaussian", presets)).toBeNull();
  // Input left to right, output bottom to top.
  expect(contourPath([0, 1, 0], 10)).toBe("M0 10 L5 0 L10 10");
  expect(contourPath([0.25, 2], 4)).toBe("M0 3 L4 0");
  expect(contourPath([1], 4)).toBe("");
});

test("the presets are asked of the engine once, again after a failure", async () => {
  ask.mockRejectedValueOnce(new Error("no engine")).mockResolvedValue(presets);
  expect(await contourPresets()).toEqual([]);
  expect(await contourPresets()).toEqual(presets);
  expect(await contourPresets()).toEqual(presets);
  expect(ask).toHaveBeenCalledTimes(2);
});

test("the presets are Photoshop's twelve, in its picker's order", () => {
  expect(CONTOUR_PRESET_IDS).toEqual([
    "linear",
    "cone",
    "coneInverted",
    "coveDeep",
    "coveShallow",
    "gaussian",
    "halfRound",
    "ring",
    "ringDouble",
    "rollingSlopeDescending",
    "roundedSteps",
    "sawtooth",
  ]);
});

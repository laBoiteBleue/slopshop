import { expect, test } from "vitest";
import type { LayerView, StackEntryView } from "../src/lib/engine";
import {
  FILTERS,
  FILTER_MENU,
  applyFilterEdit,
  filterEntryEdit,
  filterSteps,
  filterable,
  hasSettings,
  sliderPosition,
  sliderValue,
  validValues,
  withNewSeeds,
  type NumberParam,
} from "../src/lib/filters";
import { editableEntry } from "../src/lib/stackEntries";

const pixels = { kind: "raster", visible: true } as LayerView;

test("Gaussian Blur takes a radius of 0.1 to 1000 pixels, 1 at first", () => {
  expect(FILTERS.gaussianBlur.defaults).toEqual([1]);
  expect(validValues("gaussianBlur", [0.1])).toBe(true);
  expect(validValues("gaussianBlur", [1000])).toBe(true);
  for (const values of [[0], [1001], [Number.NaN], [], [1, 2]]) {
    expect(validValues("gaussianBlur", values)).toBe(false);
  }
});

test("Unsharp Mask: amount 1 to 500 %, radius 0.1 to 1000 pixels, threshold 0 to 255 levels", () => {
  expect(FILTERS.unsharpMask.defaults).toEqual([100, 1, 0]);
  expect(FILTERS.unsharpMask.params.map((p) => p.key)).toEqual(["amount", "radius", "threshold"]);
  expect(validValues("unsharpMask", [1, 0.1, 0])).toBe(true);
  expect(validValues("unsharpMask", [500, 1000, 255])).toBe(true);
  for (const values of [
    [0, 1, 0],
    [501, 1, 0],
    [100, 0, 0],
    [100, 1, -1],
    [100, 1, 256],
    [100, 1],
  ]) {
    expect(validValues("unsharpMask", values)).toBe(false);
  }
});

test("Motion Blur: an angle of -90 to 90 degrees, a distance of 1 to 2000 pixels", () => {
  expect(FILTERS.motionBlur.defaults).toEqual([0, 10]);
  expect(validValues("motionBlur", [-90, 1])).toBe(true);
  expect(validValues("motionBlur", [90, 2000])).toBe(true);
  for (const values of [
    [91, 10],
    [0, 0.5],
    [0, 2001],
  ]) {
    expect(validValues("motionBlur", values)).toBe(false);
  }
  // The angle's slider is even, its middle the horizontal.
  expect(sliderValue(FILTERS.motionBlur.params[0] as NumberParam, 500)).toBe(0);
});

test("Add Noise: an amount of 0.1 to 400 %, uniform or Gaussian, monochromatic or not, a seed", () => {
  expect(FILTERS.addNoise.defaults).toEqual([12.5, 0, 0, 0]);
  expect(validValues("addNoise", [0.1, 1, 1, 2 ** 24 - 1])).toBe(true);
  for (const values of [
    [0, 0, 0, 0],
    [401, 0, 0, 0],
    [10, 2, 0, 0],
    [10, 0.5, 0, 0],
    [10, 0, 2, 0],
    [10, 0, 0, 1.5],
    [10, 0, 0, 2 ** 24],
  ]) {
    expect(validValues("addNoise", values)).toBe(false);
  }
});

test("Dust & Scratches: a whole radius of 1 to 500 pixels, a threshold of 0 to 255 levels", () => {
  expect(FILTERS.dustAndScratches.defaults).toEqual([1, 0]);
  expect(validValues("dustAndScratches", [500, 255])).toBe(true);
  for (const values of [
    [0, 0],
    [1.5, 0],
    [501, 0],
    [3, 256],
  ]) {
    expect(validValues("dustAndScratches", values)).toBe(false);
  }
});

test("Clarity and Texture: two strengths of -100 to 100, nothing at first", () => {
  expect(FILTERS.clarityTexture.defaults).toEqual([0, 0]);
  expect(validValues("clarityTexture", [-100, 100])).toBe(true);
  expect(validValues("clarityTexture", [-101, 0])).toBe(false);
  expect(validValues("clarityTexture", [0, 100.5])).toBe(false);
  // Even sliders, 0 in the middle.
  expect(sliderValue(FILTERS.clarityTexture.params[1] as NumberParam, 500)).toBe(0);
});

test("applied anew, a filter draws new seeds and keeps its other settings", () => {
  expect(withNewSeeds("addNoise", [30, 1, 0, 5], () => 0.5)).toEqual([30, 1, 0, 2 ** 23]);
  expect(withNewSeeds("addNoise", [30, 1, 0, 5], () => 0.999999999)).toEqual([
    30,
    1,
    0,
    2 ** 24 - 1,
  ]);
  // No seed: as it was.
  expect(withNewSeeds("unsharpMask", [100, 1, 0], () => 0.5)).toEqual([100, 1, 0]);
});

test("High Pass takes a radius of 0.1 to 1000 pixels, 10 at first", () => {
  expect(FILTERS.highPass.defaults).toEqual([10]);
  expect(validValues("highPass", [0.1])).toBe(true);
  expect(validValues("highPass", [0])).toBe(false);
});

test("every filter's defaults are valid, and every filter is in a submenu once", () => {
  const ids = Object.keys(FILTERS) as (keyof typeof FILTERS)[];
  for (const id of ids) expect(validValues(id, FILTERS[id].defaults)).toBe(true);
  expect(FILTER_MENU.flatMap((menu) => menu.filters).sort()).toEqual([...ids].sort());
});

test("sliders: a radius moves by ratios, an amount or a threshold evenly", () => {
  const [amount, radius, threshold] = FILTERS.unsharpMask.params as NumberParam[];
  // 1 pixel is a quarter of the way from 0.1 to 1000, 10 halfway.
  expect(sliderPosition(radius, 1)).toBeCloseTo(250, 6);
  expect(sliderValue(radius, 500)).toBe(10);
  expect(sliderPosition(threshold, 0)).toBe(0);
  expect(sliderPosition(threshold, 255)).toBe(1000);
  expect(sliderValue(threshold, 500)).toBe(128);
  expect(sliderValue(amount, 0)).toBe(1);
  expect(sliderValue(amount, 1000)).toBe(500);
  // Out of range values sit at the ends.
  expect(sliderPosition(amount, 900)).toBe(1000);
});

test("a filter applies to the active pixel layer shown, its pixels the target, outside Quick Mask", () => {
  expect(filterable(pixels, false, false)).toBe(true);
  expect(filterable(null, false, false)).toBe(false);
  expect(filterable({ ...pixels, visible: false }, false, false)).toBe(false);
  expect(filterable({ ...pixels, kind: "adjustment" }, false, false)).toBe(false);
  expect(filterable({ ...pixels, kind: "group" }, false, false)).toBe(false);
  expect(filterable(pixels, true, false)).toBe(false);
  expect(filterable(pixels, false, true)).toBe(false);
});

test("the edits that apply a filter and set a filter entry", () => {
  const settings = { filter: "gaussianBlur" as const, values: [4] };
  expect(applyFilterEdit(3, settings)).toEqual({
    kind: "applyFilter",
    id: 3,
    filter: "gaussianBlur",
    values: [4],
  });
  const entry: StackEntryView = {
    kind: "filter",
    adjustment: null,
    count: 2,
    hidden: false,
    steps: [],
    filter: "gaussianBlur",
    filterSteps: [
      { id: "gaussianBlur", values: [2] },
      { id: "gaussianBlur", values: [5] },
    ],
  };
  expect(editableEntry(entry)).toBe(true);
  const steps = filterSteps(entry);
  expect(steps).toEqual([
    { filter: "gaussianBlur", values: [2] },
    { filter: "gaussianBlur", values: [5] },
  ]);
  expect(filterEntryEdit(3, 1, steps, true)).toEqual({
    kind: "setStackEntry",
    id: 3,
    index: 1,
    hidden: true,
    filters: steps,
  });
});

test("Box Blur, Median, Maximum and Minimum: a whole radius, up to 2000 or 500 pixels", () => {
  expect(FILTERS.boxBlur.defaults).toEqual([10]);
  expect(validValues("boxBlur", [2000])).toBe(true);
  expect(validValues("boxBlur", [2001])).toBe(false);
  for (const filter of ["median", "maximum", "minimum"] as const) {
    expect(FILTERS[filter].defaults).toEqual([1]);
    expect(validValues(filter, [1])).toBe(true);
    expect(validValues(filter, [500])).toBe(true);
    for (const values of [[0], [1.5], [501], [1, 1]]) {
      expect(validValues(filter, values)).toBe(false);
    }
  }
});

test("Emboss: an angle, a height of 1 to 10 pixels, an amount; Mosaic: cells of 2 to 200", () => {
  expect(FILTERS.emboss.defaults).toEqual([135, 3, 100]);
  expect(validValues("emboss", [-180, 10, 500])).toBe(true);
  for (const values of [
    [181, 3, 100],
    [0, 2.5, 100],
    [0, 11, 100],
    [0, 3, 0],
  ]) {
    expect(validValues("emboss", values)).toBe(false);
  }
  expect(FILTERS.mosaic.defaults).toEqual([10]);
  expect(validValues("mosaic", [2])).toBe(true);
  expect(validValues("mosaic", [1])).toBe(false);
  expect(validValues("mosaic", [201])).toBe(false);
});

test("Find Edges and Solarize have no settings: applied at once, nothing to edit again", () => {
  for (const filter of ["findEdges", "solarize"] as const) {
    expect(hasSettings(filter)).toBe(false);
    expect(validValues(filter, [])).toBe(true);
    const entry: StackEntryView = {
      kind: "filter",
      adjustment: null,
      count: 1,
      hidden: false,
      steps: [],
      filter,
      filterSteps: [{ id: filter, values: [] }],
    };
    expect(editableEntry(entry)).toBe(false);
  }
  expect(hasSettings("emboss")).toBe(true);
});

test("the distortions: Twirl's angle, Pinch's and Spherize's amounts, Polar's way, Offset's", () => {
  expect(FILTERS.twirl.defaults).toEqual([50]);
  expect(validValues("twirl", [-999])).toBe(true);
  expect(validValues("twirl", [1000])).toBe(false);
  expect(validValues("pinch", [-100])).toBe(true);
  expect(validValues("pinch", [101])).toBe(false);
  // Spherize's amount, then its mode: Normal, Horizontal only, Vertical only.
  expect(FILTERS.spherize.defaults).toEqual([100, 0]);
  expect(validValues("spherize", [-100, 2])).toBe(true);
  expect(validValues("spherize", [101, 0])).toBe(false);
  expect(validValues("spherize", [50, 3])).toBe(false);
  expect(FILTERS.polarCoordinates.defaults).toEqual([1]);
  expect(validValues("polarCoordinates", [2])).toBe(false);
  expect(FILTERS.offset.defaults).toEqual([0, 0, 2]);
  expect(validValues("offset", [-30000, 30000, 0])).toBe(true);
  expect(validValues("offset", [0.5, 0, 0])).toBe(false);
  expect(validValues("offset", [0, 0, 3])).toBe(false);
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.distort")?.filters).toEqual([
    "pinch",
    "polarCoordinates",
    "ripple",
    "spherize",
    "twirl",
    "wave",
    "zigZag",
  ]);
});

test("Stylize, Pixelate and HSB/HSL: their settings and the filters without", () => {
  expect(FILTERS.wind.defaults).toEqual([0, 0, 0]);
  expect(validValues("wind", [2, 1, 9])).toBe(true);
  expect(validValues("wind", [3, 0, 0])).toBe(false);
  expect(validValues("diffuse", [3, 0])).toBe(true);
  expect(validValues("diffuse", [4, 0])).toBe(false);
  expect(FILTERS.traceContour.defaults).toEqual([128, 1]);
  expect(validValues("traceContour", [256, 1])).toBe(false);
  expect(FILTERS.crystallize.defaults).toEqual([10, 0]);
  expect(validValues("crystallize", [2, 0])).toBe(false);
  expect(validValues("crystallize", [300, 0])).toBe(true);
  expect(validValues("mezzotint", [9, 0])).toBe(true);
  expect(validValues("mezzotint", [10, 0])).toBe(false);
  expect(FILTERS.colorHalftone.defaults).toEqual([8, 108, 162, 90, 45]);
  expect(validValues("colorHalftone", [127, -360, 360, 0, 0])).toBe(true);
  expect(validValues("colorHalftone", [3, 0, 0, 0, 0])).toBe(false);
  expect(FILTERS.hsbHsl.defaults).toEqual([0, 1]);
  expect(hasSettings("facet")).toBe(false);
  expect(hasSettings("fragment")).toBe(false);
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.pixelate")?.filters).toEqual([
    "colorHalftone",
    "crystallize",
    "facet",
    "fragment",
    "mezzotint",
    "mosaic",
  ]);
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.stylize")?.filters).toEqual([
    "diffuse",
    "emboss",
    "findEdges",
    "solarize",
    "traceContour",
    "wind",
  ]);
});

test("Ripple, ZigZag and Wave: Photoshop's ranges and defaults", () => {
  expect(FILTERS.ripple.defaults).toEqual([100, 1]);
  expect(validValues("ripple", [-999, 0])).toBe(true);
  expect(validValues("ripple", [1000, 0])).toBe(false);
  expect(validValues("ripple", [100, 3])).toBe(false);
  expect(FILTERS.zigZag.defaults).toEqual([10, 5, 2]);
  expect(validValues("zigZag", [-100, 20, 0])).toBe(true);
  expect(validValues("zigZag", [10, 0, 0])).toBe(false);
  expect(validValues("zigZag", [10, 2.5, 0])).toBe(false);
  // Generators, wavelengths, amplitudes, scales, type, undefined areas, seed.
  expect(FILTERS.wave.defaults).toEqual([5, 10, 120, 5, 35, 100, 100, 0, 1, 0]);
  expect(validValues("wave", FILTERS.wave.defaults)).toBe(true);
  expect(validValues("wave", [999, 1, 999, 1, 999, 1, 1, 2, 0, 7])).toBe(true);
  expect(validValues("wave", [0, 10, 120, 5, 35, 100, 100, 0, 1, 0])).toBe(false);
  expect(validValues("wave", [5, 10, 120, 5, 35, 101, 100, 0, 1, 0])).toBe(false);
  expect(validValues("wave", [5, 10, 120, 5, 35, 100, 100, 0, 2, 0])).toBe(false);
  // Each application draws another seed, the other settings kept.
  const again = withNewSeeds("wave", FILTERS.wave.defaults, () => 0.5);
  expect(again).toEqual([5, 10, 120, 5, 35, 100, 100, 0, 1, 2 ** 23]);
});

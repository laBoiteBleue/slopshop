import { expect, test } from "vitest";
import type { LayerView, StackEntryView } from "../src/lib/engine";
import {
  ADJUSTMENT_FILTERS,
  FILTERS,
  FILTER_MENU,
  SEEDS,
  applyFilterEdit,
  applyFiltersEdit,
  isAdjustmentFilter,
  shownParams,
  centerAt,
  filterEntryEdit,
  filterSteps,
  filterable,
  hasSettings,
  newSeed,
  nudgedCenter,
  sliderPosition,
  sliderValue,
  validValues,
  withColors,
  withNewSeeds,
  packColor,
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

test("every filter's defaults are valid, and every filter is in a submenu or Image > Adjustments once", () => {
  const ids = Object.keys(FILTERS) as (keyof typeof FILTERS)[];
  for (const id of ids) expect(validValues(id, FILTERS[id].defaults)).toBe(true);
  expect([...FILTER_MENU.flatMap((menu) => menu.filters), ...ADJUSTMENT_FILTERS].sort()).toEqual(
    [...ids].sort(),
  );
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

test("Ripple, Wave and ZigZag: Photoshop's settings, ranges and defaults", () => {
  expect(FILTERS.spherize.defaults).toEqual([100, 0]);
  expect(FILTERS.ripple.defaults).toEqual([100, 1]);
  expect(validValues("ripple", [-999, 0])).toBe(true);
  expect(validValues("ripple", [1000, 1])).toBe(false);
  expect(validValues("ripple", [10.5, 1])).toBe(false);
  expect(validValues("ripple", [10, 3])).toBe(false);
  expect(FILTERS.wave.params.map((p) => p.key)).toEqual([
    "generators",
    "wavelengthMin",
    "wavelengthMax",
    "amplitudeMin",
    "amplitudeMax",
    "scaleH",
    "scaleV",
    "type",
    "edge",
    "seed",
  ]);
  expect(FILTERS.wave.defaults).toEqual([5, 10, 120, 5, 35, 100, 100, 0, 1, 0]);
  // Min above max is taken the other way round, as the engine does.
  expect(validValues("wave", [999, 120, 10, 35, 5, 1, 100, 2, 0, SEEDS - 1])).toBe(true);
  expect(validValues("wave", [0, 10, 120, 5, 35, 100, 100, 0, 1, 0])).toBe(false);
  expect(validValues("wave", [5, 10, 120, 5, 35, 101, 100, 0, 1, 0])).toBe(false);
  expect(validValues("wave", [5, 10, 120, 5, 35, 100, 100, 0, 2, 0])).toBe(false);
  // Its seed drawn anew when applied, and by Randomize.
  const again = withNewSeeds("wave", FILTERS.wave.defaults, () => 0.5);
  expect(again).toEqual([5, 10, 120, 5, 35, 100, 100, 0, 1, SEEDS / 2]);
  expect(newSeed(() => 0.25)).toBe(SEEDS / 4);
  expect(FILTERS.zigZag.defaults).toEqual([10, 5, 2]);
  expect(validValues("zigZag", [-100, 20, 0])).toBe(true);
  expect(validValues("zigZag", [10, 21, 0])).toBe(false);
  expect(validValues("zigZag", [101, 5, 0])).toBe(false);
});

test("the Blur menu: Photoshop's order, Average applied at once", () => {
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.blur")?.filters).toEqual([
    "average",
    "boxBlur",
    "gaussianBlur",
    "motionBlur",
    "radialBlur",
    "smartBlur",
    "surfaceBlur",
  ]);
  expect(hasSettings("average")).toBe(false);
  expect(validValues("average", [])).toBe(true);
});

test("Radial Blur: amount 1 to 100, Spin or Zoom, Draft to Best, a center in percent", () => {
  expect(FILTERS.radialBlur.defaults).toEqual([10, 0, 1, 50, 50]);
  expect(FILTERS.radialBlur.params.map((p) => p.key)).toEqual([
    "amount",
    "method",
    "quality",
    "centerX",
    "centerY",
  ]);
  expect(validValues("radialBlur", [1, 1, 2, 0, 100])).toBe(true);
  for (const values of [
    [0, 0, 1, 50, 50],
    [101, 0, 1, 50, 50],
    [10.5, 0, 1, 50, 50],
    [10, 2, 1, 50, 50],
    [10, 0, 3, 50, 50],
    [10, 0, 1, -1, 50],
    [10, 0, 1, 50, 101],
    [10, 0, 1, 50],
  ]) {
    expect(validValues("radialBlur", values)).toBe(false);
  }
});

test("Surface Blur: a radius of 1 to 100 whole pixels, a threshold of 2 to 255 levels", () => {
  expect(FILTERS.surfaceBlur.defaults).toEqual([5, 15]);
  expect(validValues("surfaceBlur", [1, 2])).toBe(true);
  expect(validValues("surfaceBlur", [100, 255])).toBe(true);
  for (const values of [
    [0, 15],
    [101, 15],
    [2.5, 15],
    [5, 1],
    [5, 256],
  ]) {
    expect(validValues("surfaceBlur", values)).toBe(false);
  }
});

test("a center picked in its square: in whole percents, within it; arrows nudge it", () => {
  const box = { left: 100, top: 50, width: 120, height: 120 };
  expect(centerAt(box, 160, 110)).toEqual([50, 50]);
  expect(centerAt(box, 100, 170)).toEqual([0, 100]);
  expect(centerAt(box, 0, 500)).toEqual([0, 100]);
  expect(centerAt(box, 130, 80)).toEqual([25, 25]);
  expect(centerAt({ left: 0, top: 0, width: 0, height: 0 }, 5, 5)).toEqual([50, 50]);
  expect(nudgedCenter([50, 50], "ArrowLeft", false)).toEqual([49, 50]);
  expect(nudgedCenter([50, 50], "ArrowDown", true)).toEqual([50, 60]);
  expect(nudgedCenter([95, 3], "ArrowRight", true)).toEqual([100, 3]);
  expect(nudgedCenter([95, 3], "ArrowUp", true)).toEqual([95, 0]);
  expect(nudgedCenter([50, 50], "Enter", false)).toBeNull();
});

test("Smart Blur: radius and threshold of 0.1 to 100, Low to High, Normal or its edges", () => {
  expect(FILTERS.smartBlur.defaults).toEqual([3, 25, 0, 0]);
  expect(validValues("smartBlur", [0.1, 0.1, 2, 2])).toBe(true);
  expect(validValues("smartBlur", [100, 100, 0, 1])).toBe(true);
  for (const values of [
    [0, 25, 0, 0],
    [101, 25, 0, 0],
    [3, 0, 0, 0],
    [3, 101, 0, 0],
    [3, 25, 3, 0],
    [3, 25, 0, 3],
  ]) {
    expect(validValues("smartBlur", values)).toBe(false);
  }
});

test("Shadows/Highlights: Photoshop's settings and defaults, the basic ones without More Options", () => {
  expect(FILTERS.shadowsHighlights.defaults).toEqual([35, 50, 30, 0, 50, 30, 20, 0]);
  expect(validValues("shadowsHighlights", [100, 100, 2500, 0, 0, 0, -100, 100])).toBe(true);
  for (const values of [
    [101, 50, 30, 0, 50, 30, 20, 0],
    [35, 50, 2501, 0, 50, 30, 20, 0],
    [35, 50, 30, 0, -1, 30, 20, 0],
    [35, 50, 30, 0, 50, 30, 20, 101],
    [35, 50, 30.5, 0, 50, 30, 20, 0],
  ]) {
    expect(validValues("shadowsHighlights", values)).toBe(false);
  }
  // Shadows' and Highlights' Amounts only, then everything.
  expect(shownParams("shadowsHighlights", false)).toEqual([0, 3]);
  expect(shownParams("shadowsHighlights", true)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  expect(shownParams("unsharpMask", false)).toEqual([0, 1, 2]);
  expect(FILTERS.shadowsHighlights.more).toBe(true);
  // In Image > Adjustments, not in the Filter menu.
  expect(isAdjustmentFilter("shadowsHighlights")).toBe(true);
  expect(isAdjustmentFilter("gaussianBlur")).toBe(false);
  expect(ADJUSTMENT_FILTERS).toEqual(["shadowsHighlights"]);
  expect(FILTER_MENU.some((m) => m.filters.includes("shadowsHighlights"))).toBe(false);
});

test("an adjustment filter is applied to every layer given, in one batch", () => {
  const settings = { filter: "shadowsHighlights" as const, values: [35, 50, 30, 0, 50, 30, 20, 0] };
  expect(applyFiltersEdit([4], settings)).toEqual(applyFilterEdit(4, settings));
  expect(applyFiltersEdit([4, 7], settings)).toEqual({
    kind: "batch",
    edits: [applyFilterEdit(4, settings), applyFilterEdit(7, settings)],
  });
});

test("Render: Clouds and Difference Clouds apply at once; Fibers' Variance and Strength", () => {
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.render")?.filters).toEqual([
    "clouds",
    "differenceClouds",
    "fibers",
  ]);
  // Black and white at first; the colors and a seed hidden.
  expect(FILTERS.clouds.defaults).toEqual([0, 0xffffff, 0]);
  expect(FILTERS.differenceClouds.defaults).toEqual([0, 0xffffff, 0]);
  expect(FILTERS.fibers.defaults).toEqual([16, 4, 0, 0xffffff, 0]);
  expect(hasSettings("clouds")).toBe(false);
  expect(hasSettings("differenceClouds")).toBe(false);
  expect(hasSettings("fibers")).toBe(true);
  expect(validValues("fibers", [1, 64, 0xffffff, 0, SEEDS - 1])).toBe(true);
  for (const values of [
    [0, 4, 0, 0, 0],
    [16, 65, 0, 0, 0],
    [16, 4.5, 0, 0, 0],
    [16, 4, 2 ** 24, 0, 0],
    [16, 4, -1, 0, 0],
    [16, 4, 0.5, 0, 0],
  ]) {
    expect(validValues("fibers", values)).toBe(false);
  }
  const entry = (filter: "clouds" | "fibers", values: number[]): StackEntryView => ({
    kind: "filter",
    adjustment: null,
    count: 1,
    hidden: false,
    steps: [],
    filter,
    filterSteps: [{ id: filter, values }],
  });
  expect(editableEntry(entry("clouds", [0, 0xffffff, 3]))).toBe(false);
  expect(editableEntry(entry("fibers", FILTERS.fibers.defaults))).toBe(true);
});

test("a drawing filter takes the colors of the moment, packed, and a new seed", () => {
  expect(packColor("#123456")).toBe(0x123456);
  const colors = { foreground: "#ff0000", background: "#0000ff" };
  expect(withColors("clouds", [0, 0xffffff, 7], colors)).toEqual([0xff0000, 0x0000ff, 7]);
  expect(withColors("fibers", [20, 8, 0, 0, 7], colors)).toEqual([20, 8, 0xff0000, 0x0000ff, 7]);
  // A filter without colors keeps its values.
  expect(withColors("gaussianBlur", [4], colors)).toEqual([4]);
  expect(withNewSeeds("clouds", [1, 2, 7], () => 0.5)).toEqual([1, 2, 2 ** 23]);
});

test("Stylize: Photoshop's order, Diffuse's and Wind's choices, Tiles, Trace Contour, Oil Paint", () => {
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.stylize")?.filters).toEqual([
    "diffuse",
    "emboss",
    "findEdges",
    "oilPaint",
    "solarize",
    "tiles",
    "traceContour",
    "wind",
  ]);
  expect(FILTERS.diffuse.defaults).toEqual([0, 0]);
  expect(validValues("diffuse", [3, 5])).toBe(true);
  expect(validValues("diffuse", [4, 5])).toBe(false);
  expect(FILTERS.wind.defaults).toEqual([0, 0, 0]);
  expect(validValues("wind", [2, 1, 0])).toBe(true);
  expect(validValues("wind", [3, 0, 0])).toBe(false);
  // Tiles: 10 tiles, 10 %, the background color by default; the colors of the moment.
  expect(FILTERS.tiles.defaults).toEqual([10, 10, 0, 0, 0xffffff, 0]);
  expect(validValues("tiles", [99, 1, 3, 0, 0, 0])).toBe(true);
  for (const values of [
    [0, 10, 0, 0, 0, 0],
    [10, 100, 0, 0, 0, 0],
    [10, 10, 4, 0, 0, 0],
  ]) {
    expect(validValues("tiles", values)).toBe(false);
  }
  const colors = { foreground: "#102030", background: "#405060" };
  expect(withColors("tiles", FILTERS.tiles.defaults, colors)).toEqual([
    10, 10, 0, 0x102030, 0x405060, 0,
  ]);
  expect(FILTERS.traceContour.defaults).toEqual([128, 1]);
  expect(validValues("traceContour", [255, 0])).toBe(true);
  expect(validValues("traceContour", [256, 0])).toBe(false);
  // Oil Paint: Stylization and Scale from 0.1, the others from 0, to 10; Lighting a check box.
  expect(FILTERS.oilPaint.defaults).toEqual([4, 5, 1, 2, 1, -60, 1]);
  expect(validValues("oilPaint", [0.1, 0, 0.1, 0, 0, -180, 0])).toBe(true);
  for (const values of [
    [0, 5, 1, 2, 1, -60, 1],
    [4, 10.5, 1, 2, 1, -60, 1],
    [4, 5, 1, 2, 2, -60, 1],
    [4, 5, 1, 2, 1, 181, 1],
  ]) {
    expect(validValues("oilPaint", values)).toBe(false);
  }
  for (const filter of ["diffuse", "wind", "tiles", "traceContour", "oilPaint"] as const) {
    expect(hasSettings(filter)).toBe(true);
  }
});

test("Pixelate: Photoshop's order and settings; Facet and Fragment apply at once", () => {
  expect(FILTER_MENU.find((m) => m.label === "menu.filter.pixelate")?.filters).toEqual([
    "colorHalftone",
    "crystallize",
    "facet",
    "fragment",
    "mezzotint",
    "mosaic",
    "pointillize",
  ]);
  for (const filter of ["facet", "fragment"] as const) {
    expect(hasSettings(filter)).toBe(false);
  }
  for (const filter of ["colorHalftone", "crystallize", "mezzotint", "pointillize"] as const) {
    expect(hasSettings(filter)).toBe(true);
  }
  // Color Halftone: Max. Radius 4 to 127, three screen angles of -360 to 360 degrees.
  expect(FILTERS.colorHalftone.defaults).toEqual([8, 108, 162, 90]);
  expect(validValues("colorHalftone", [4, -360, 360, 0])).toBe(true);
  for (const values of [
    [3, 108, 162, 90],
    [128, 108, 162, 90],
    [8, 361, 162, 90],
    [8, 108, 162.5, 90],
  ]) {
    expect(validValues("colorHalftone", values)).toBe(false);
  }
  // Crystallize and Pointillize: cells of 3 to 300, a seed; Pointillize's background color.
  expect(FILTERS.crystallize.defaults).toEqual([10, 0]);
  expect(FILTERS.pointillize.defaults).toEqual([5, 0xffffff, 0]);
  for (const filter of ["crystallize", "pointillize"] as const) {
    const [, ...rest] = FILTERS[filter].defaults;
    expect(validValues(filter, [3, ...rest])).toBe(true);
    expect(validValues(filter, [300, ...rest])).toBe(true);
    expect(validValues(filter, [2, ...rest])).toBe(false);
    expect(validValues(filter, [301, ...rest])).toBe(false);
  }
  const colors = { foreground: "#102030", background: "#405060" };
  expect(withColors("pointillize", [5, 0xffffff, 7], colors)).toEqual([5, 0x405060, 7]);
  expect(withNewSeeds("crystallize", [10, 7], () => 0.5)).toEqual([10, 2 ** 23]);
  // Mezzotint: ten types, Fine Dots first.
  expect(FILTERS.mezzotint.defaults).toEqual([0, 0]);
  expect(validValues("mezzotint", [9, 0])).toBe(true);
  expect(validValues("mezzotint", [10, 0])).toBe(false);
});

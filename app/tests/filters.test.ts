import { expect, test } from "vitest";
import type { LayerView, StackEntryView } from "../src/lib/engine";
import {
  FILTERS,
  FILTER_MENU,
  applyFilterEdit,
  filterEntryEdit,
  filterSteps,
  filterable,
  sliderPosition,
  sliderValue,
  validValues,
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
  const [amount, radius, threshold] = FILTERS.unsharpMask.params;
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

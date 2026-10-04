import { expect, test } from "vitest";
import type { LayerView, StackEntryView } from "../src/lib/engine";
import {
  FILTERS,
  applyFilterEdit,
  filterEntryEdit,
  filterSteps,
  filterable,
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
